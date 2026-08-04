//! 分类框液态玻璃 GPU 渲染器（对齐 liquid-glass-rs/examples 实现）
//!
//! 使用 DXGI Desktop Duplication 捕获显示器画面，通过 D2D Effect Graph
//! (Crop -> Border -> GaussianBlur -> ColorMatrix) 处理纹理，再通过自定义
//! PSMain shader 采样处理后的纹理和法线贴图生成预乘 alpha BGRA 位图。

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::mem::size_of;
use std::slice;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HMODULE, POINT, RECT as WinRect};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COMPOSITE_MODE_SOURCE_COPY, D2D1_PIXEL_FORMAT, D2D_MATRIX_5X4_F,
    D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1Border, CLSID_D2D1ColorMatrix, CLSID_D2D1Crop, CLSID_D2D1GaussianBlur,
    D2D1CreateDevice, ID2D1DeviceContext, ID2D1Image, D2D1_BITMAP_OPTIONS_NONE,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_BORDER_EDGE_MODE_MIRROR,
    D2D1_BORDER_PROP_EDGE_MODE_X, D2D1_BORDER_PROP_EDGE_MODE_Y, D2D1_COLORMATRIX_PROP_COLOR_MATRIX,
    D2D1_CROP_PROP_RECT, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_MATRIX_5X4,
    D2D1_PROPERTY_TYPE_VECTOR4,
};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, IDXGISurface,
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::ffi_types::Pixel32;
use crate::config::LiquidGlassConfig;

const RETRY_DELAY: Duration = Duration::from_secs(3);
const MAX_SURFACE_CACHE_ENTRIES: usize = 8;

const STANDARD_MAP: &[u8] = include_bytes!("embedded/standard.jpg");
const POLAR_MAP: &[u8] = include_bytes!("embedded/polar.jpg");
const PROMINENT_MAP: &[u8] = include_bytes!("embedded/prominent.png");

const SHADER: &str = r#"
cbuffer C : register(b0) {
 float2 outputOrigin, outputSize, windowOrigin, windowSize, lensSize, padding;
 float4 shadowSettings, refractionSettings, effectsSettings, colorSettings, detailSettings, controlSettings;
 float2 mouseOffset, interactionPadding;
};
Texture2D desktop : register(t0);
Texture2D displacementMap : register(t1);
SamplerState samp : register(s0);
struct V { float4 pos:SV_POSITION; float2 uv:TEXCOORD0; };
V VSMain(uint id:SV_VertexID) { V o; float2 p=float2((id<<1)&2,id&2); o.uv=p; o.pos=float4(p*float2(2,-2)+float2(-1,1),0,1); return o; }
float sdf(float2 p,float2 halfSize,float radius) { float2 q=abs(p)-halfSize+radius; return min(max(q.x,q.y),0)+length(max(q,0))-radius; }
float gaussianTail(float distance,float sigma) {
 float x=max(distance,0)/(max(sigma,.001)*1.41421356);
 float t=1/(1+.3275911*x);
 float erfValue=1-(((((1.061405429*t-1.453152027)*t+1.421413741)*t-.284496736)*t+.254829592)*t)*exp(-x*x);
 return .5*(1-erfValue);
}
float2 proceduralMapTexel(float2 pixel,float2 size,float maxScale) {
 pixel=clamp(pixel,float2(0,0),size-1);
 float2 uv=pixel/size,center=uv-.5;
 float mapDistance=sdf(center,float2(.3,.2),.6);
 float displacement=smoothstep(.8,0,mapDistance-.15);
 float scaled=smoothstep(0,1,displacement);
 float2 rawOffset=(center*scaled+.5-uv)*size;
 float edgeDistance=min(min(pixel.x,pixel.y),min(size.x-pixel.x-1,size.y-pixel.y-1));
 float edgeFactor=saturate(edgeDistance/2);
 float2 encoded=clamp(rawOffset/max(maxScale,1),-.5,.5)*edgeFactor;
 return round(saturate(encoded+.5)*255)/255-.5;
}
float2 sampleProceduralMap(float2 local,float2 physicalSize,float dpiScale,float maxScale) {
 float2 size=physicalSize/dpiScale;
 float2 source=local/dpiScale-.5;
 float2 base=floor(source),fraction=frac(source);
 float2 top=lerp(proceduralMapTexel(base,size,maxScale),proceduralMapTexel(base+float2(1,0),size,maxScale),fraction.x);
 float2 bottom=lerp(proceduralMapTexel(base+float2(0,1),size,maxScale),proceduralMapTexel(base+float2(1,1),size,maxScale),fraction.x);
 return lerp(top,bottom,fraction.y);
}
float gradientAlpha(float position,float stopA,float stopB,float alphaA,float alphaB) {
 if(position<=stopA) return lerp(0,alphaA,saturate(position/max(stopA,.0001)));
 if(position<=stopB) return lerp(alphaA,alphaB,saturate((position-stopA)/max(stopB-stopA,.0001)));
 return lerp(alphaB,0,saturate((position-stopB)/max(1-stopB,.0001)));
}
float3 overlayWhite(float3 backdrop) {
 return float3(backdrop.r<=.5 ? backdrop.r*2 : 1,backdrop.g<=.5 ? backdrop.g*2 : 1,backdrop.b<=.5 ? backdrop.b*2 : 1);
}
float3 linearToSrgb(float3 value) {
 value=saturate(value);
 float3 low=value*12.92;
 float3 high=1.055*pow(value,1/2.4)-.055;
 return lerp(low,high,step(.0031308,value));
}
float3 cssSaturate(float3 value,float amount) {
 return float3(dot(value,float3(.213+.787*amount,.715-.715*amount,.072-.072*amount)),dot(value,float3(.213-.213*amount,.715+.285*amount,.072-.072*amount)),dot(value,float3(.213-.213*amount,.715-.715*amount,.072+.928*amount)));
}
float2 mirrorSource(float2 uv,float2 minimum,float2 maximum,float2 texel) {
 if(uv.x<minimum.x) uv.x=2*minimum.x-uv.x;
 if(uv.y<minimum.y) uv.y=2*minimum.y-uv.y;
 if(uv.x>maximum.x) uv.x=2*maximum.x-uv.x;
 if(uv.y>maximum.y) uv.y=2*maximum.y-uv.y;
 return clamp(uv,minimum+texel*.5,maximum-texel*.5);
}
float3 sampleChromatic(float2 baseUv,float2 offsetPx,float2 texel,float aberration,float baseScale,float2 sourceMinimum,float2 sourceMaximum) {
 float2 redUv=mirrorSource(baseUv+offsetPx*baseScale*texel,sourceMinimum,sourceMaximum,texel);
 float2 greenUv=mirrorSource(baseUv+offsetPx*(baseScale-aberration*.05)*texel,sourceMinimum,sourceMaximum,texel);
 float2 blueUv=mirrorSource(baseUv+offsetPx*(baseScale-aberration*.10)*texel,sourceMinimum,sourceMaximum,texel);
 return float3(desktop.Sample(samp,redUv).r,desktop.Sample(samp,greenUv).g,desktop.Sample(samp,blueUv).b);
}
float4 PSMain(V i):SV_TARGET {
 float2 px=i.uv*windowSize, local=px-padding, p=local-lensSize*.5;
 float mouseLength=length(mouseOffset);
 float2 mouseDirection=mouseOffset/max(mouseLength,.001);
 float2 mouseOutside=max(abs(mouseOffset)-lensSize*.5,0);
 float mouseFade=saturate(1-length(mouseOutside)/max(200*controlSettings.y,.001));
 float stretch=min(mouseLength/max(300*controlSettings.y,.001),1)*controlSettings.z*mouseFade;
 float2 shapeScale=float2(1+abs(mouseDirection.x)*stretch*.3-abs(mouseDirection.y)*stretch*.15,1+abs(mouseDirection.y)*stretch*.3-abs(mouseDirection.x)*stretch*.15);
 float2 shapeP=p/shapeScale;
 float radius=min(lensSize.y*.5,controlSettings.w);
 float d=sdf(shapeP,lensSize*.5,radius); float sd=sdf(shapeP-float2(0,shadowSettings.x),lensSize*.5,radius);
 float shadow=gaussianTail(sd,shadowSettings.y*.5)*shadowSettings.z;
 float2 uv=local/lensSize;
 float2 encodedOffset=sampleProceduralMap(local,lensSize,controlSettings.y,colorSettings.z/controlSettings.y);
 bool proceduralMode=interactionPadding.y>=2.5;
 float mapAspect=lensSize.x/max(lensSize.y,.001);
 float2 mapUv=uv;
 if(mapAspect>=1) mapUv.y=(uv.y-.5)/mapAspect+.5; else mapUv.x=(uv.x-.5)*mapAspect+.5;
 float4 mapSample=displacementMap.Sample(samp,mapUv);
 float2 staticOffset=float2(mapSample.r,mapSample.b)-.5;
 float2 offsetPx=(proceduralMode ? encodedOffset : staticOffset)*effectsSettings.y;
 float2 baseScreenUv=(windowOrigin+padding+local-outputOrigin)/outputSize, texel=1/outputSize;
 float2 sourceMinimum=(windowOrigin+padding-outputOrigin)/outputSize;
 float2 sourceMaximum=(windowOrigin+padding+lensSize-outputOrigin)/outputSize;
 float postSigma=effectsSettings.w;
 float neighborWeight=exp(-.5/max(postSigma*postSigma,.0001));
 float diagonalWeight=neighborWeight*neighborWeight;
 float weightSum=1+neighborWeight*4+diagonalWeight*4;
 float baseScale=proceduralMode ? 1 : -1;
 float3 col=sampleChromatic(baseScreenUv,offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum);
 col+=sampleChromatic(baseScreenUv+float2(texel.x,0),offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*neighborWeight;
 col+=sampleChromatic(baseScreenUv-float2(texel.x,0),offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*neighborWeight;
 col+=sampleChromatic(baseScreenUv+float2(0,texel.y),offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*neighborWeight;
 col+=sampleChromatic(baseScreenUv-float2(0,texel.y),offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*neighborWeight;
 col+=sampleChromatic(baseScreenUv+texel,offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*diagonalWeight;
 col+=sampleChromatic(baseScreenUv-texel,offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*diagonalWeight;
 col+=sampleChromatic(baseScreenUv+float2(texel.x,-texel.y),offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*diagonalWeight;
 col+=sampleChromatic(baseScreenUv+float2(-texel.x,texel.y),offsetPx,texel,effectsSettings.z,baseScale,sourceMinimum,sourceMaximum)*diagonalWeight;
 col/=weightSum;
 if(interactionPadding.x>.5) col=saturate(cssSaturate(linearToSrgb(col),colorSettings.x));
 float2 normal=float2(sdf(shapeP+float2(1,0),lensSize*.5,radius)-sdf(shapeP-float2(1,0),lensSize*.5,radius),sdf(shapeP+float2(0,1),lensSize*.5,radius)-sdf(shapeP-float2(0,1),lensSize*.5,radius));
 normal=normalize(normal+float2(.0001,.0001));
 float2 normalizedMouse=float2(mouseOffset.x/max(lensSize.x,1),mouseOffset.y/max(lensSize.y,1))*100;
 float angle=radians(135+normalizedMouse.x*1.2);
 float2 gradientDirection=float2(sin(angle),-cos(angle));
 float gradientExtent=max(dot(abs(gradientDirection),lensSize*.5),.0001);
 float gradientPosition=dot(local-lensSize*.5,gradientDirection)/(gradientExtent*2)+.5;
 float stopA=clamp((33+normalizedMouse.y*.3)/100,.1,.9);
 float stopB=clamp((66+normalizedMouse.y*.4)/100,.1,.9);
 float insideDistance=max(-d,0);
 float borderWidth=1.5*controlSettings.y;
 float borderMask=saturate(1-smoothstep(0,borderWidth,insideDistance));
 float insetDirection=saturate(.5-normal.y*.5);
 float insetGlow=exp(-.5*pow(insideDistance/max(borderWidth,.001),2))*insetDirection*.25;
 float insetAlpha=saturate((1-smoothstep(.25*controlSettings.y,.75*controlSettings.y,insideDistance))*.5+insetGlow)*borderMask;
 float screenAlpha=saturate(gradientAlpha(gradientPosition,stopA,stopB,.12+abs(normalizedMouse.x)*.008,.4+abs(normalizedMouse.x)*.012))*borderMask*.2;
 screenAlpha=1-(1-screenAlpha)*(1-insetAlpha*.2);
 col=1-(1-col)*(1-screenAlpha);
 float overlayAlpha=saturate(gradientAlpha(gradientPosition,stopA,stopB,.32+abs(normalizedMouse.x)*.008,.6+abs(normalizedMouse.x)*.012))*borderMask;
 overlayAlpha=1-(1-overlayAlpha)*(1-insetAlpha);
 col=lerp(col,overlayWhite(col),overlayAlpha);
 float aa=max(fwidth(d)*detailSettings.z,detailSettings.w);
 float glassAlpha=smoothstep(aa,-aa,d);
 float alpha=glassAlpha+shadow*(1-glassAlpha);
 return float4(col*glassAlpha,alpha);
}"#;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Constants {
    output_origin: [f32; 2],
    output_size: [f32; 2],
    window_origin: [f32; 2],
    window_size: [f32; 2],
    lens_size: [f32; 2],
    padding: [f32; 2],
    shadow_settings: [f32; 4],
    refraction_settings: [f32; 4],
    effects_settings: [f32; 4],
    color_settings: [f32; 4],
    detail_settings: [f32; 4],
    control_settings: [f32; 4],
    mouse_offset: [f32; 2],
    interaction_padding: [f32; 2],
}

struct CaptureOutput {
    rect: WinRect,
    output1: IDXGIOutput1,
    duplication: IDXGIOutputDuplication,
    hdr_active: bool,
    desktop_texture: Option<ID3D11Texture2D>,
    desktop_view: Option<ID3D11ShaderResourceView>,
    blurred_texture: Option<ID3D11Texture2D>,
    blurred_view: Option<ID3D11ShaderResourceView>,
}

struct EmbeddedMap {
    view: ID3D11ShaderResourceView,
}

struct RenderSurfaces {
    width: u32,
    height: u32,
    output_texture: ID3D11Texture2D,
    output_target: ID3D11RenderTargetView,
    staging_texture: ID3D11Texture2D,
}

struct GlassRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    d2d_context: ID2D1DeviceContext,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    constants: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    rasterizer: ID3D11RasterizerState,
    outputs: Vec<CaptureOutput>,
    surfaces: HashMap<(u32, u32), RenderSurfaces>,
    displacement_maps: [EmbeddedMap; 3],
    light_direction: [f32; 2],
}

enum GlassState {
    Uninitialized,
    Ready(GlassRenderer),
    Failed { retry_at: Instant },
}

thread_local! {
    static GLASS_STATE: RefCell<GlassState> = const { RefCell::new(GlassState::Uninitialized) };
}

static LIQUID_GLASS_CONFIG: std::sync::OnceLock<std::sync::RwLock<LiquidGlassConfig>> =
    std::sync::OnceLock::new();

fn get_config_lock() -> &'static std::sync::RwLock<LiquidGlassConfig> {
    LIQUID_GLASS_CONFIG.get_or_init(|| std::sync::RwLock::new(LiquidGlassConfig::default()))
}

pub fn update_liquid_glass_config(config: &LiquidGlassConfig) {
    if let Ok(mut guard) = get_config_lock().write() {
        let mut normalized = config.clone();
        normalized.normalize();
        *guard = normalized;
    }
}

pub fn get_liquid_glass_config() -> LiquidGlassConfig {
    get_config_lock()
        .read()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn render_background(
    screen_x: i32,
    screen_y: i32,
    content_width: i32,
    content_height: i32,
    padding: i32,
    corner_radius: f32,
    dpi_scale: f32,
    glass_config: &LiquidGlassConfig,
) -> Option<Vec<Pixel32>> {
    if content_width <= 0 || content_height <= 0 || padding < 0 {
        return None;
    }

    log::debug!(
        "[LG] render_background called: {}x{}+{} padding={}",
        content_width,
        content_height,
        screen_x,
        padding
    );
    GLASS_STATE.with(|state| {
        let mut state = match state.try_borrow_mut() {
            Ok(s) => s,
            Err(_) => {
                log::debug!("液态玻璃渲染器当前忙碌，跳过本帧");
                return None;
            }
        };
        if matches!(&*state, GlassState::Failed { retry_at } if Instant::now() < *retry_at) {
            return None;
        }

        if !matches!(&*state, GlassState::Ready(_)) {
            match GlassRenderer::new() {
                Ok(renderer) => {
                    log::info!("液态玻璃 D3D11 渲染器已初始化");
                    *state = GlassState::Ready(renderer);
                }
                Err(error) => {
                    log::warn!("液态玻璃初始化失败，使用半透明背景: {error:#}");
                    *state = GlassState::Failed {
                        retry_at: Instant::now() + RETRY_DELAY,
                    };
                    return None;
                }
            }
        }

        let GlassState::Ready(renderer) = &mut *state else {
            return None;
        };

        match renderer.render(
            screen_x,
            screen_y,
            content_width as u32,
            content_height as u32,
            padding as u32,
            corner_radius,
            dpi_scale,
            glass_config,
        ) {
            Ok(pixels) => {
                log::debug!("[LG] render OK, {} pixels", pixels.len());
                Some(pixels)
            }
            Err(error) => {
                log::warn!("液态玻璃帧渲染失败，回退半透明背景: {error:#}");
                *state = GlassState::Failed {
                    retry_at: Instant::now() + RETRY_DELAY,
                };
                None
            }
        }
    })
}

fn compile_shader(entry: &[u8], target: &[u8]) -> Result<ID3DBlob> {
    unsafe {
        let mut code = None;
        let mut errors: Option<ID3DBlob> = None;
        D3DCompile(
            SHADER.as_ptr() as *const c_void,
            SHADER.len(),
            None,
            None,
            None::<&windows::Win32::Graphics::Direct3D::ID3DInclude>,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
        .map_err(|e| {
            if let Some(errors) = errors {
                let msg = CStr::from_ptr(errors.GetBufferPointer() as *const i8).to_string_lossy();
                anyhow!("Shader compilation failed: {}\n{}", e, msg)
            } else {
                anyhow!("Shader compilation failed: {}", e)
            }
        })?;
        code.context("Shader compilation returned no code")
    }
}

fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe { slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()) }
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    unsafe {
        let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];
        let mut device = None;
        let mut context = None;
        let mut chosen = D3D_FEATURE_LEVEL::default();
        windows::Win32::Graphics::Direct3D11::D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut chosen),
            Some(&mut context),
        )?;
        Ok((
            device.context("创建 D3D11 设备未返回对象")?,
            context.context("创建 D3D11 设备上下文未返回对象")?,
        ))
    }
}

fn create_outputs(device: &ID3D11Device) -> Result<Vec<CaptureOutput>> {
    unsafe {
        let dxgi: IDXGIDevice = device.cast()?;
        let adapter = dxgi.GetAdapter()?;

        let mut outputs = Vec::new();
        let mut i = 0;
        loop {
            let output = match adapter.EnumOutputs(i) {
                Ok(o) => o,
                Err(_) => break,
            };
            i += 1;
            let output1 = output.cast::<IDXGIOutput1>()?;
            let duplication = output1.DuplicateOutput(device)?;
            let desc = output.GetDesc()?;
            outputs.push(CaptureOutput {
                rect: desc.DesktopCoordinates,
                output1: output1.clone(),
                duplication,
                hdr_active: false,
                desktop_texture: None,
                desktop_view: None,
                blurred_texture: None,
                blurred_view: None,
            });
        }
        Ok(outputs)
    }
}

fn decode_embedded_map(
    wic: &windows::Win32::Graphics::Imaging::IWICImagingFactory,
    data: &[u8],
) -> Result<(Vec<u8>, u32, u32)> {
    use windows::Win32::Graphics::Imaging::{
        GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
    };

    unsafe {
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(data)?;

        let decoder = wic.CreateDecoderFromStream(
            &stream,
            std::ptr::null(),
            windows::Win32::Graphics::Imaging::WICDecodeMetadataCacheOnLoad,
        )?;
        let frame = decoder.GetFrame(0)?;

        let mut width = 0u32;
        let mut height = 0u32;
        frame.GetSize(&mut width, &mut height)?;

        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let stride = width * 4;
        let mut pixels = vec![0u8; (height * stride) as usize];
        converter.CopyPixels(std::ptr::null(), stride, &mut pixels)?;

        Ok((pixels, width, height))
    }
}

fn upload_embedded_map(
    device: &ID3D11Device,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<EmbeddedMap> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };

        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const c_void,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        };

        let mut texture = None;
        device.CreateTexture2D(&desc, Some(&initial), Some(&mut texture))?;
        let texture = texture.context("创建法线贴图纹理未返回对象")?;

        let mut view = None;
        device.CreateShaderResourceView(&texture, None, Some(&mut view))?;
        let view = view.context("创建法线贴图视图未返回对象")?;

        Ok(EmbeddedMap { view })
    }
}

fn point_in_rect(x: i32, y: i32, rect: &WinRect) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

impl GlassRenderer {
    fn new() -> Result<Self> {
        unsafe {
            let (device, context) = create_device()?;

            let dxgi_device: IDXGIDevice = device.cast()?;
            let d2d_device = D2D1CreateDevice(&dxgi_device, None)?;
            let d2d_context = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

            let wic = windows::Win32::System::Com::CoCreateInstance(
                &windows::Win32::Graphics::Imaging::CLSID_WICImagingFactory,
                None,
                windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
            )?;

            let (std_pixels, std_w, std_h) = decode_embedded_map(&wic, STANDARD_MAP)?;
            let (pol_pixels, pol_w, pol_h) = decode_embedded_map(&wic, POLAR_MAP)?;
            let (pro_pixels, pro_w, pro_h) = decode_embedded_map(&wic, PROMINENT_MAP)?;

            let std_map = upload_embedded_map(&device, &std_pixels, std_w, std_h)?;
            let pol_map = upload_embedded_map(&device, &pol_pixels, pol_w, pol_h)?;
            let pro_map = upload_embedded_map(&device, &pro_pixels, pro_w, pro_h)?;

            let vertex_blob = compile_shader(b"VSMain\0", b"vs_5_0\0")?;
            let pixel_blob = compile_shader(b"PSMain\0", b"ps_5_0\0")?;

            let mut vertex_shader = None;
            device.CreateVertexShader(blob_bytes(&vertex_blob), None, Some(&mut vertex_shader))?;
            let mut pixel_shader = None;
            device.CreatePixelShader(blob_bytes(&pixel_blob), None, Some(&mut pixel_shader))?;

            let buffer_desc = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<Constants>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut constants = None;
            device.CreateBuffer(&buffer_desc, None, Some(&mut constants))?;

            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            device.CreateSamplerState(&sampler_desc, Some(&mut sampler))?;

            let mut rasterizer = None;
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    ScissorEnable: true.into(),
                    DepthClipEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut rasterizer),
            )?;

            let outputs = create_outputs(&device)?;
            if outputs.is_empty() {
                return Err(anyhow!("未找到可用的 DXGI 显示输出"));
            }

            Ok(Self {
                device,
                context,
                d2d_context,
                vertex_shader: vertex_shader.context("创建顶点着色器未返回对象")?,
                pixel_shader: pixel_shader.context("创建像素着色器未返回对象")?,
                constants: constants.context("创建常量缓冲区未返回对象")?,
                sampler: sampler.context("创建采样器未返回对象")?,
                rasterizer: rasterizer.context("创建光栅化器未返回对象")?,
                outputs,
                surfaces: HashMap::new(),
                displacement_maps: [std_map, pol_map, pro_map],
                light_direction: [0.0, 0.0],
            })
        }
    }

    /// Rebuild the IDXGIOutputDuplication for a specific output after DXGI_ERROR_ACCESS_LOST.
    /// This is much cheaper than recreating the entire GlassRenderer and avoids
    /// the 3-second fallback to semi-transparent background during window resize.
    fn rebuild_duplication(&mut self, index: usize) -> Result<()> {
        unsafe {
            let output = &mut self.outputs[index];
            // Release any held frame (best-effort; may already be released)
            let _ = output.duplication.ReleaseFrame();
            // Create a new duplication from the stored IDXGIOutput1 reference
            let new_duplication = output.output1.DuplicateOutput(&self.device)?;
            output.duplication = new_duplication;
            // Clear cached textures so they get recreated with the new duplication
            output.desktop_texture = None;
            output.desktop_view = None;
            output.blurred_texture = None;
            output.blurred_view = None;
            log::info!("液态玻璃 DXGI Output Duplication 已重建 (index={})", index);
            Ok(())
        }
    }

    fn capture_output(&mut self, index: usize) -> Result<()> {
        unsafe {
            let output = &mut self.outputs[index];
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            let acquire = output
                .duplication
                .AcquireNextFrame(0, &mut frame_info, &mut resource);

            if let Err(e) = acquire {
                if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                    log::debug!(
                        "[LG] capture WAIT_TIMEOUT, desktop_texture={}",
                        output.desktop_texture.is_some()
                    );
                    return Ok(());
                }
                if e.code() == DXGI_ERROR_ACCESS_LOST {
                    // DXGI desktop capture access lost — rebuild duplication instead of
                    // failing the entire renderer. This is common during rapid window resize.
                    return self.rebuild_duplication(index);
                }
                return Err(e.into());
            }

            let Some(resource) = resource else {
                let _ = output.duplication.ReleaseFrame();
                return Ok(());
            };
            let texture = resource.cast::<ID3D11Texture2D>()?;

            let mut frame_desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut frame_desc);
            output.hdr_active = matches!(
                frame_desc.Format,
                DXGI_FORMAT_R10G10B10A2_UNORM | DXGI_FORMAT_R16G16B16A16_FLOAT
            );

            if output.desktop_texture.is_none() {
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                let new_desc = D3D11_TEXTURE2D_DESC {
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                    ..desc
                };
                let mut new_texture = None;
                self.device
                    .CreateTexture2D(&new_desc, None, Some(&mut new_texture))?;
                let new_texture = new_texture.context("创建桌面纹理未返回对象")?;
                let mut view = None;
                self.device
                    .CreateShaderResourceView(&new_texture, None, Some(&mut view))?;
                let view = view.context("创建桌面纹理视图未返回对象")?;
                output.desktop_texture = Some(new_texture);
                output.desktop_view = Some(view);
            }

            if let Some(ref dest) = output.desktop_texture {
                self.context.CopyResource(dest, &texture);
            }
            let _ = output.duplication.ReleaseFrame();
            Ok(())
        }
    }

    fn blur_output_impl(
        device: &ID3D11Device,
        d2d_context: &ID2D1DeviceContext,
        output: &mut CaptureOutput,
        sigma: f32,
        saturation: f32,
        lens_x: i32,
        lens_y: i32,
        lens_w: i32,
        lens_h: i32,
    ) -> Result<()> {
        unsafe {
            let Some(ref source_texture) = output.desktop_texture else {
                return Ok(());
            };

            let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
            source_texture.GetDesc(&mut texture_desc);

            if output.blurred_texture.is_none() {
                let new_desc = D3D11_TEXTURE2D_DESC {
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                    ..texture_desc
                };
                let mut new_texture = None;
                device.CreateTexture2D(&new_desc, None, Some(&mut new_texture))?;
                let new_texture = new_texture.context("创建模糊纹理未返回对象")?;
                let mut view = None;
                device.CreateShaderResourceView(&new_texture, None, Some(&mut view))?;
                let view = view.context("创建模糊纹理视图未返回对象")?;
                output.blurred_texture = Some(new_texture);
                output.blurred_view = Some(view);
            }

            let Some(ref target_texture) = output.blurred_texture else {
                return Ok(());
            };

            let saturation = if output.hdr_active { 1.0 } else { saturation };
            let pixel_format = D2D1_PIXEL_FORMAT {
                format: texture_desc.Format,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            };

            let source_properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: pixel_format,
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                ..Default::default()
            };

            let target_properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: pixel_format,
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                ..Default::default()
            };

            let source_surface = source_texture.cast::<IDXGISurface>()?;
            let target_surface = target_texture.cast::<IDXGISurface>()?;

            let source_bitmap = d2d_context
                .CreateBitmapFromDxgiSurface(&source_surface, Some(&source_properties))?;
            let target_bitmap = d2d_context
                .CreateBitmapFromDxgiSurface(&target_surface, Some(&target_properties))?;

            let crop_rect = D2D_RECT_F {
                left: (lens_x - output.rect.left).max(0) as f32,
                top: (lens_y - output.rect.top).max(0) as f32,
                right: (lens_x + lens_w - output.rect.left).min(texture_desc.Width as i32) as f32,
                bottom: (lens_y + lens_h - output.rect.top).min(texture_desc.Height as i32) as f32,
            };
            if crop_rect.right <= crop_rect.left || crop_rect.bottom <= crop_rect.top {
                return Ok(());
            }

            let crop = d2d_context.CreateEffect(&CLSID_D2D1Crop)?;
            crop.SetInput(0, &source_bitmap, true);
            let crop_bytes = slice::from_raw_parts(
                (&crop_rect as *const D2D_RECT_F).cast::<u8>(),
                size_of::<D2D_RECT_F>(),
            );
            crop.SetValue(
                D2D1_CROP_PROP_RECT.0 as u32,
                D2D1_PROPERTY_TYPE_VECTOR4,
                crop_bytes,
            )?;
            let cropped = crop.GetOutput()?;

            let border = d2d_context.CreateEffect(&CLSID_D2D1Border)?;
            border.SetInput(0, &cropped, true);
            let edge_mode = D2D1_BORDER_EDGE_MODE_MIRROR.0;
            let edge_bytes =
                slice::from_raw_parts((&edge_mode as *const i32).cast::<u8>(), size_of::<i32>());
            border.SetValue(
                D2D1_BORDER_PROP_EDGE_MODE_X.0 as u32,
                D2D1_PROPERTY_TYPE_ENUM,
                edge_bytes,
            )?;
            border.SetValue(
                D2D1_BORDER_PROP_EDGE_MODE_Y.0 as u32,
                D2D1_PROPERTY_TYPE_ENUM,
                edge_bytes,
            )?;
            let bordered = border.GetOutput()?;

            let blur = d2d_context.CreateEffect(&CLSID_D2D1GaussianBlur)?;
            blur.SetInput(0, &bordered, true);
            let sigma_bytes = slice::from_raw_parts((&sigma as *const f32).cast::<u8>(), 4);
            blur.SetValue(
                D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                D2D1_PROPERTY_TYPE_FLOAT,
                sigma_bytes,
            )?;
            let blurred_image = blur.GetOutput()?;

            let saturate_effect = d2d_context.CreateEffect(&CLSID_D2D1ColorMatrix)?;
            saturate_effect.SetInput(0, &blurred_image, true);
            let matrix = D2D_MATRIX_5X4_F {
                Anonymous: windows::Win32::Graphics::Direct2D::Common::D2D_MATRIX_5X4_F_0 {
                    m: [
                        0.213 + 0.787 * saturation,
                        0.213 - 0.213 * saturation,
                        0.213 - 0.213 * saturation,
                        0.0,
                        0.715 - 0.715 * saturation,
                        0.715 + 0.285 * saturation,
                        0.715 - 0.715 * saturation,
                        0.0,
                        0.072 - 0.072 * saturation,
                        0.072 - 0.072 * saturation,
                        0.072 + 0.928 * saturation,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        1.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                    ],
                },
            };
            let saturation_bytes = slice::from_raw_parts(
                (&matrix as *const D2D_MATRIX_5X4_F).cast::<u8>(),
                size_of::<D2D_MATRIX_5X4_F>(),
            );
            saturate_effect.SetValue(
                D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32,
                D2D1_PROPERTY_TYPE_MATRIX_5X4,
                saturation_bytes,
            )?;
            let saturated_image = saturate_effect.GetOutput()?;

            d2d_context.SetTarget(&target_bitmap);
            d2d_context.BeginDraw();
            d2d_context.DrawImage(
                &saturated_image,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_COPY,
            );
            let _ = d2d_context.EndDraw(None, None);
            d2d_context.SetTarget(None::<&ID2D1Image>);
            Ok(())
        }
    }

    fn ensure_surfaces(&mut self, width: u32, height: u32) -> Result<()> {
        if self.surfaces.contains_key(&(width, height)) {
            return Ok(());
        }

        unsafe {
            let output_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };

            let mut output_texture = None;
            self.device
                .CreateTexture2D(&output_desc, None, Some(&mut output_texture))?;
            let output_texture = output_texture.context("创建输出纹理未返回对象")?;

            let mut output_target = None;
            self.device
                .CreateRenderTargetView(&output_texture, None, Some(&mut output_target))?;
            let output_target = output_target.context("创建输出渲染目标视图未返回对象")?;

            let staging_desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..output_desc
            };
            let mut staging_texture = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging_texture))?;
            let staging_texture = staging_texture.context("创建暂存纹理未返回对象")?;

            self.surfaces.insert(
                (width, height),
                RenderSurfaces {
                    width,
                    height,
                    output_texture,
                    output_target,
                    staging_texture,
                },
            );

            // Evict oldest entries AFTER insertion, never evict the current key.
            // HashMap iteration order is non-deterministic, so we explicitly skip
            // the key we just inserted to avoid use-after-evict.
            while self.surfaces.len() > MAX_SURFACE_CACHE_ENTRIES {
                let evict_key = self
                    .surfaces
                    .keys()
                    .copied()
                    .find(|k| *k != (width, height));
                if let Some(k) = evict_key {
                    log::debug!("[LG] evict surface cache: {:?}", k);
                    self.surfaces.remove(&k);
                } else {
                    break;
                }
            }
            Ok(())
        }
    }

    fn render(
        &mut self,
        screen_x: i32,
        screen_y: i32,
        content_width: u32,
        content_height: u32,
        padding: u32,
        corner_radius: f32,
        dpi_scale: f32,
        glass_config: &LiquidGlassConfig,
    ) -> Result<Vec<Pixel32>> {
        let center_x = screen_x + padding as i32 + content_width as i32 / 2;
        let center_y = screen_y + padding as i32 + content_height as i32 / 2;

        let output_index = self
            .outputs
            .iter()
            .position(|output| point_in_rect(center_x, center_y, &output.rect))
            .ok_or_else(|| anyhow!("分类框中心不在当前 D3D11 适配器的任何输出中"))?;

        self.capture_output(output_index)?;
        let blur_sigma = ((if glass_config.over_light { 12.0 } else { 4.0 })
            + glass_config.blur_amount * 32.0)
            * dpi_scale;
        let saturation = glass_config.saturation;

        // Capture and blur in separate scope to avoid borrow conflicts
        {
            let output = &mut self.outputs[output_index];
            Self::blur_output_impl(
                &self.device,
                &self.d2d_context,
                output,
                blur_sigma,
                saturation,
                screen_x + padding as i32,
                screen_y + padding as i32,
                content_width as i32,
                content_height as i32,
            )?;
        }

        let output = &self.outputs[output_index];
        let blurred_view = output
            .blurred_view
            .as_ref()
            .context("尚未取得模糊帧")?
            .clone();
        let output_rect = output.rect;
        let hdr_active = output.hdr_active;

        let mut cursor = POINT::default();
        let target_light = unsafe {
            if GetCursorPos(&mut cursor).is_ok() {
                [
                    cursor.x as f32
                        - (screen_x as f32 + padding as f32 + content_width as f32 * 0.5),
                    cursor.y as f32
                        - (screen_y as f32 + padding as f32 + content_height as f32 * 0.5),
                ]
            } else {
                [0.0, 0.0]
            }
        };
        const LIGHT_BLEND: f32 = 0.14;
        self.light_direction[0] += (target_light[0] - self.light_direction[0]) * LIGHT_BLEND;
        self.light_direction[1] += (target_light[1] - self.light_direction[1]) * LIGHT_BLEND;

        self.render_view(
            &blurred_view,
            output_rect,
            screen_x,
            screen_y,
            content_width,
            content_height,
            padding,
            corner_radius,
            dpi_scale,
            hdr_active,
            glass_config,
        )
    }

    fn render_view(
        &mut self,
        blurred_view: &ID3D11ShaderResourceView,
        output_rect: WinRect,
        screen_x: i32,
        screen_y: i32,
        content_width: u32,
        content_height: u32,
        padding: u32,
        corner_radius: f32,
        dpi_scale: f32,
        hdr_active: bool,
        glass_config: &LiquidGlassConfig,
    ) -> Result<Vec<Pixel32>> {
        let surface_width = content_width + padding * 2;
        let surface_height = content_height + padding * 2;
        self.ensure_surfaces(surface_width, surface_height)?;

        let output_width = (output_rect.right - output_rect.left).max(1) as f32;
        let output_height = (output_rect.bottom - output_rect.top).max(1) as f32;

        // 示例的第 4 个模式是 shader 内生成的程序化贴图。
        let mode = glass_config.refraction_mode.min(2) as usize;
        let map_view = &self.displacement_maps[mode].view;

        let constants = Constants {
            output_origin: [output_rect.left as f32, output_rect.top as f32],
            output_size: [output_width, output_height],
            window_origin: [screen_x as f32, screen_y as f32],
            window_size: [surface_width as f32, surface_height as f32],
            lens_size: [content_width as f32, content_height as f32],
            padding: [padding as f32, padding as f32],
            shadow_settings: [
                (if glass_config.over_light { 16.0 } else { 12.0 }) * dpi_scale,
                (if glass_config.over_light { 70.0 } else { 40.0 }) * dpi_scale,
                if glass_config.over_light { 0.75 } else { 0.25 },
                0.0,
            ],
            refraction_settings: [0.30, 0.20, 0.60, 0.80],
            effects_settings: [
                0.15,
                glass_config.displacement_scale
                    * if glass_config.over_light { 0.5 } else { 1.0 }
                    * dpi_scale,
                glass_config.chromatic_offset,
                (0.5 - glass_config.chromatic_offset * 0.1).max(0.1) * dpi_scale,
            ],
            color_settings: [
                if hdr_active {
                    1.0
                } else {
                    glass_config.saturation
                },
                if glass_config.over_light { 0.0 } else { 0.018 },
                content_width as f32 * 0.324_668_2,
                0.07,
            ],
            detail_settings: [0.34, 0.08, 0.75, 0.75],
            control_settings: [
                if glass_config.over_light { 1.0 } else { 0.0 },
                dpi_scale,
                glass_config.elasticity,
                corner_radius.min(content_height as f32 * 0.5) * dpi_scale,
            ],
            mouse_offset: self.light_direction,
            interaction_padding: [
                if hdr_active { 1.0 } else { 0.0 },
                glass_config.refraction_mode as f32,
            ],
        };

        unsafe {
            let surfaces = self
                .surfaces
                .get(&(surface_width, surface_height))
                .ok_or_else(|| {
                    log::warn!(
                        "[LG] surfaces.get FAILED for {}x{}, cache_len={}",
                        surface_width,
                        surface_height,
                        self.surfaces.len()
                    );
                    anyhow::anyhow!("渲染表面未创建")
                })?;
            self.context
                .OMSetRenderTargets(Some(&[Some(surfaces.output_target.clone())]), None);
            self.context
                .ClearRenderTargetView(&surfaces.output_target, &[0.0; 4]);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: surface_width as f32,
                Height: surface_height as f32,
                MaxDepth: 1.0,
                ..Default::default()
            }]));
            self.context.RSSetState(&self.rasterizer);
            self.context.IASetInputLayout(None);
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(
                &self.constants,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            mapped.pData.cast::<Constants>().write(constants);
            self.context.Unmap(&self.constants, 0);

            // 与 frosted_liquid 示例一致：启用 scissor 后必须在 Draw 前设置范围。
            // 分类框的液态玻璃表面就是完整窗口，因此裁剪范围为整张输出纹理。
            self.context.RSSetScissorRects(Some(&[WinRect {
                left: 0,
                top: 0,
                right: surface_width as i32,
                bottom: surface_height as i32,
            }]));
            self.context.PSSetShaderResources(
                0,
                Some(&[Some(blurred_view.clone()), Some(map_view.clone())]),
            );
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&[None, None]));

            self.context
                .CopyResource(&surfaces.staging_texture, &surfaces.output_texture);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(
                &surfaces.staging_texture,
                0,
                D3D11_MAP_READ,
                0,
                Some(&mut mapped),
            )?;

            let mut pixels = vec![
                Pixel32 {
                    b: 0,
                    g: 0,
                    r: 0,
                    a: 0
                };
                (surface_width * surface_height) as usize
            ];
            let row_bytes = surface_width as usize * size_of::<Pixel32>();

            for row in 0..surface_height as usize {
                let source = mapped
                    .pData
                    .cast::<u8>()
                    .add(row * mapped.RowPitch as usize);
                let destination = pixels.as_mut_ptr().cast::<u8>().add(row * row_bytes);
                std::ptr::copy_nonoverlapping(source, destination, row_bytes);
            }

            self.context.Unmap(&surfaces.staging_texture, 0);
            let center_index = ((padding + content_height / 2) * surface_width
                + padding
                + content_width / 2) as usize;
            if pixels.get(center_index).is_none_or(|pixel| pixel.a == 0) {
                return Err(anyhow!("液态玻璃 GPU 输出中心像素完全透明"));
            }
            Ok(pixels)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_layout_matches_frosted_liquid_example() {
        // 40 个 f32；必须与 liquid-glass-rs 的 cbuffer C 字段顺序一致。
        assert_eq!(size_of::<Constants>(), 160);
    }

    #[test]
    fn shader_includes_all_example_refraction_modes() {
        assert!(SHADER.contains("proceduralMode=interactionPadding.y>=2.5"));
        assert!(SHADER.contains("sampleChromatic"));
        assert!(SHADER.contains("controlSettings.z"));
    }

    #[test]
    fn shader_renders_outer_shadow_into_padded_surface() {
        assert!(SHADER.contains("shadow*(1-glassAlpha)"));
        assert!(SHADER.contains("local=px-padding"));
        assert!(SHADER.contains("lensSize"));
    }
}
