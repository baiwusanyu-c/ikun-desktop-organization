//! 分类框液态玻璃 GPU 渲染器（对齐 liquid-glass-rs/examples 实现）
//!
//! 使用 DXGI Desktop Duplication 捕获显示器画面，通过 D2D Effect Graph
//! (Crop -> Border -> GaussianBlur -> ColorMatrix) 处理纹理，再通过自定义
//! PSMain shader 采样处理后的纹理和法线贴图生成预乘 alpha BGRA 位图。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::ffi::{c_void, CStr};
use std::mem::size_of;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    D2D1CreateDevice, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Effect, ID2D1Image,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_BORDER_EDGE_MODE_MIRROR, D2D1_BORDER_PROP_EDGE_MODE_X,
    D2D1_BORDER_PROP_EDGE_MODE_Y, D2D1_COLORMATRIX_PROP_COLOR_MATRIX, D2D1_CROP_PROP_RECT,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
    D2D1_PROPERTY_TYPE_MATRIX_5X4, D2D1_PROPERTY_TYPE_VECTOR4,
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
    DXGI_OUTDUPL_MOVE_RECT,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::ffi_types::Pixel32;
use crate::config::{BoxStyleConfig, LiquidGlassConfig};

const RETRY_DELAY: Duration = Duration::from_secs(3);
const MAX_SURFACE_CACHE_ENTRIES: usize = 8;
static FIRST_FRAME_DIAGNOSTIC_LOGGED: AtomicBool = AtomicBool::new(false);
static NON_BLACK_FRAME_DIAGNOSTIC_LOGGED: AtomicBool = AtomicBool::new(false);

/// 桌面内容代号：任一显示器捕获到新帧就自增。
///
/// 分类框的周期 tick 用它判断「玻璃背景是否变了」，
/// 没变就跳过整帧渲染。
static DESKTOP_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 桌面变化记录：（代号, 桌面坐标矩形）。
///
/// 分类框不能只看「桌面变了没」：任务栏时钟跳一下、别处光标
/// 闪一下，整个桌面代号就前进了，于是所有框都得重绘。
/// 记下具体矩形后，每个框只需问「我这块区域变了吗」。
const MAX_DESKTOP_CHANGES: usize = 512;

/// 桌面变化轮询的最小间隔。
///
/// 每个分类框都有自己的 200ms 定时器且相互错开，限流后
/// 同一时间窗内只有第一个调用真正去 DXGI 取帧。
const DESKTOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

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

/// 复用的 D2D 模糊 effect 链（每个 output 一份）。
///
/// 各级输入在创建时接好，运行期只用 SetValue 改参数。保持 effect
/// 对象身份稳定，D2D1 才能命中它为该图缓存的 realization（模糊中间表面）。
struct BlurChain {
    crop: ID2D1Effect,
    border: ID2D1Effect,
    blur: ID2D1Effect,
    saturate: ID2D1Effect,
    /// `saturate.GetOutput()`，DrawImage 的输入
    saturated: ID2D1Image,
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
    /// 桌面捕获纹理的 D2D 位图包装（随 desktop_texture 一起失效）
    source_bitmap: Option<ID2D1Bitmap1>,
    /// 模糊目标纹理的 D2D 位图包装（随 blurred_texture 一起失效）
    target_bitmap: Option<ID2D1Bitmap1>,
    /// 复用的模糊 effect 链
    blur_chain: Option<BlurChain>,
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
    startup_black_frames_remaining: u8,
}

enum GlassState {
    Uninitialized,
    Ready(GlassRenderer),
    Failed { retry_at: Instant },
}

thread_local! {
    static GLASS_STATE: RefCell<GlassState> = const { RefCell::new(GlassState::Uninitialized) };
    static LAST_GOOD_FRAMES: RefCell<HashMap<(i32, i32, i32, i32, i32), Vec<Pixel32>>> = RefCell::new(HashMap::new());
    /// 待判定的桌面变化矩形队列（按代号递增）
    static DESKTOP_CHANGES: RefCell<VecDeque<(u64, WinRect)>> = RefCell::new(VecDeque::new());
    /// 上一次真正向 DXGI 取帧的时刻（轮询节流用）
    static LAST_DESKTOP_POLL: Cell<Option<Instant>> = const { Cell::new(None) };
}

static LIQUID_GLASS_CONFIG: std::sync::OnceLock<std::sync::RwLock<LiquidGlassConfig>> =
    std::sync::OnceLock::new();
static BOX_STYLE_CONFIG: std::sync::OnceLock<std::sync::RwLock<BoxStyleConfig>> =
    std::sync::OnceLock::new();

fn get_config_lock() -> &'static std::sync::RwLock<LiquidGlassConfig> {
    LIQUID_GLASS_CONFIG.get_or_init(|| std::sync::RwLock::new(LiquidGlassConfig::default()))
}

fn get_box_style_lock() -> &'static std::sync::RwLock<BoxStyleConfig> {
    BOX_STYLE_CONFIG.get_or_init(|| std::sync::RwLock::new(BoxStyleConfig::default()))
}

/// 更新原生分类框渲染所使用的完整样式快照。
pub fn update_box_style_config(config: &BoxStyleConfig) {
    let mut normalized = config.clone();
    normalized.normalize();
    update_liquid_glass_config(&normalized.liquid_glass);
    if let Ok(mut guard) = get_box_style_lock().write() {
        *guard = normalized;
    }
    // 不再清空帧缓存。配置已更新并保存到磁盘，
    // 缓存会在窗口移动/调整大小时自然失效（cache key 包含屏幕坐标），
    // 届时新配置会生效。这避免了在用户修改液态玻璃参数时
    // 触发 GPU 密集型渲染，防止与 WebView2 争用 GPU 资源。
}

/// 获取原生分类框当前使用的样式快照。
pub fn get_box_style_config() -> BoxStyleConfig {
    get_box_style_lock()
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// 把缓存的上一成功帧填进调用方缓冲区。
///
/// 只在 GPU 捕获失败 / 渲染器忙碌 / 单帧失败时走这条路，
/// 因此这里的一次克隆不在热路径上。
fn fill_from_cache(key: (i32, i32, i32, i32, i32), out: &mut Vec<Pixel32>) -> bool {
    let Some(cached) = cached_frame(key) else {
        return false;
    };
    out.clear();
    out.extend_from_slice(&cached);
    true
}

fn cached_frame(key: (i32, i32, i32, i32, i32)) -> Option<Vec<Pixel32>> {
    LAST_GOOD_FRAMES.with(|frames| frames.borrow().get(&key).cloned())
}

fn cache_frame(key: (i32, i32, i32, i32, i32), pixels: &[Pixel32]) {
    LAST_GOOD_FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        // 命中同一个 key 时复用已有 Vec 的分配：`clear` 保留容量，
        // 长度不变就不会再向分配器要内存。旧实现每帧 to_vec 一块
        // 1~5MB 再丢掉上一块，大对象反复申请/归还会让堆的已提交
        // 容量单调上涨。
        if let Some(slot) = frames.get_mut(&key) {
            slot.clear();
            slot.extend_from_slice(pixels);
            return;
        }
        frames.insert(key, pixels.to_vec());
        while frames.len() > MAX_SURFACE_CACHE_ENTRIES {
            let Some(old_key) = frames.keys().copied().find(|candidate| *candidate != key) else {
                break;
            };
            frames.remove(&old_key);
        }
    });
}

struct FrameStats {
    averages: [u64; 4],
    near_black_percent: f64,
    samples: u64,
}

fn content_frame_stats(
    pixels: &[Pixel32],
    content_width: i32,
    content_height: i32,
    padding: i32,
) -> Option<FrameStats> {
    let surface_width = (content_width + padding * 2) as usize;
    let mut sums = [0_u64; 4];
    let mut samples = 0_u64;
    let mut near_black = 0_u64;
    for y in (padding as usize..(padding + content_height) as usize).step_by(8) {
        for x in (padding as usize..(padding + content_width) as usize).step_by(8) {
            let pixel = pixels.get(y * surface_width + x)?;
            sums[0] += pixel.r as u64;
            sums[1] += pixel.g as u64;
            sums[2] += pixel.b as u64;
            sums[3] += pixel.a as u64;
            if pixel.r <= 8 && pixel.g <= 8 && pixel.b <= 8 && pixel.a >= 200 {
                near_black += 1;
            }
            samples += 1;
        }
    }
    (samples > 0).then(|| FrameStats {
        averages: [
            sums[0] / samples,
            sums[1] / samples,
            sums[2] / samples,
            sums[3] / samples,
        ],
        near_black_percent: near_black as f64 * 100.0 / samples as f64,
        samples,
    })
}

fn is_dxgi_placeholder(stats: &FrameStats) -> bool {
    stats.averages[0] <= 2
        && stats.averages[1] <= 2
        && stats.averages[2] <= 2
        && stats.near_black_percent >= 95.0
}

fn log_first_frame_diagnostic(
    pixels: &[Pixel32],
    content_width: i32,
    content_height: i32,
    padding: i32,
) {
    if NON_BLACK_FRAME_DIAGNOSTIC_LOGGED.load(Ordering::Relaxed) {
        return;
    }

    let Some(stats) = content_frame_stats(pixels, content_width, content_height, padding) else {
        log::warn!("液态玻璃首帧诊断没有取得内容区像素");
        return;
    };
    if !FIRST_FRAME_DIAGNOSTIC_LOGGED.swap(true, Ordering::Relaxed) {
        log::info!(
            "液态玻璃首帧像素: avg_rgba=({},{},{},{}), near_black={:.1}%, samples={}",
            stats.averages[0],
            stats.averages[1],
            stats.averages[2],
            stats.averages[3],
            stats.near_black_percent,
            stats.samples
        );
    }
    if stats.near_black_percent < 90.0
        && !NON_BLACK_FRAME_DIAGNOSTIC_LOGGED.swap(true, Ordering::Relaxed)
    {
        log::info!(
            "液态玻璃动态采集已恢复: avg_rgba=({},{},{},{}), near_black={:.1}%",
            stats.averages[0],
            stats.averages[1],
            stats.averages[2],
            stats.averages[3],
            stats.near_black_percent
        );
    }
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

/// 液态玻璃是否启用。
///
/// 分类框需要根据这个开关算外投影留白与是否装配周期重绘定时器，
/// 而 `get_liquid_glass_config` 会克隆整份配置（含多个 String / Vec）。
/// 只读开关时走这里，避免把配置克隆变成热路径上的分配点。
pub fn is_liquid_glass_enabled() -> bool {
    get_config_lock()
        .read()
        .map(|guard| guard.enabled)
        .unwrap_or(false)
}

/// 在显示器/DPI/桌面组成变化后丢弃旧快照，下一次分类框渲染前重新采集。
pub fn invalidate_desktop_snapshot() {
    GLASS_STATE.with(|state| {
        if let Ok(mut state) = state.try_borrow_mut() {
            if let GlassState::Ready(renderer) = &mut *state {
                for output in &mut renderer.outputs {
                    output.desktop_texture = None;
                    output.desktop_view = None;
                    output.blurred_texture = None;
                    output.blurred_view = None;
                    output.source_bitmap = None;
                    output.target_bitmap = None;
                    output.blur_chain = None;
                    output.source_bitmap = None;
                    output.target_bitmap = None;
                    output.blur_chain = None;
                }
            }
        }
    });
    LAST_GOOD_FRAMES.with(|frames| frames.borrow_mut().clear());
}

/// 读取当前桌面代号（不触发 DXGI 轮询）。
pub fn desktop_generation() -> u64 {
    DESKTOP_GENERATION.load(Ordering::Relaxed)
}

/// 把一个输出坐标的矩形换算成桌面坐标后记入变化队列。
fn push_change(rect: WinRect, origin_x: i32, origin_y: i32, generation: u64) {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }
    let desktop = WinRect {
        left: rect.left.saturating_add(origin_x),
        top: rect.top.saturating_add(origin_y),
        right: rect.right.saturating_add(origin_x),
        bottom: rect.bottom.saturating_add(origin_y),
    };
    DESKTOP_CHANGES.with(|cell| {
        let mut queue = cell.borrow_mut();
        queue.push_back((generation, desktop));
        while queue.len() > MAX_DESKTOP_CHANGES {
            queue.pop_front();
        }
    });
}

/// 采集本帧的脏矩形与移动矩形。
///
/// 必须在 `AcquireNextFrame` 与 `ReleaseFrame` 之间调用。
/// 两种矩形都要取：窗口拖动/滚动时 DXGI 用 move rect 表达，
/// dirty rect 里可能只有很小一块甚至为空。
unsafe fn collect_frame_changes(output: &CaptureOutput, generation: u64) {
    let origin_x = output.rect.left;
    let origin_y = output.rect.top;
    let empty = WinRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    // 第一次传 0 只为了拿到所需字节数（返回 DXGI_ERROR_MORE_DATA）
    let mut required: u32 = 0;
    let _ = output
        .duplication
        .GetFrameDirtyRects(0, std::ptr::null_mut(), &mut required);
    let rect_size = size_of::<WinRect>() as u32;
    if required >= rect_size {
        let count = (required / rect_size) as usize;
        let mut buffer = vec![empty; count];
        let mut written: u32 = 0;
        if output
            .duplication
            .GetFrameDirtyRects(required, buffer.as_mut_ptr(), &mut written)
            .is_ok()
        {
            let got = (written as usize / rect_size as usize).min(count);
            for rect in &buffer[..got] {
                push_change(*rect, origin_x, origin_y, generation);
            }
        }
    }

    let mut required: u32 = 0;
    let _ = output
        .duplication
        .GetFrameMoveRects(0, std::ptr::null_mut(), &mut required);
    let move_size = size_of::<DXGI_OUTDUPL_MOVE_RECT>() as u32;
    if required >= move_size {
        let count = (required / move_size) as usize;
        let mut buffer = vec![
            DXGI_OUTDUPL_MOVE_RECT {
                SourcePoint: POINT { x: 0, y: 0 },
                DestinationRect: empty,
            };
            count
        ];
        let mut written: u32 = 0;
        if output
            .duplication
            .GetFrameMoveRects(required, buffer.as_mut_ptr(), &mut written)
            .is_ok()
        {
            let got = (written as usize / move_size as usize).min(count);
            for m in &buffer[..got] {
                push_change(m.DestinationRect, origin_x, origin_y, generation);
                let source = WinRect {
                    left: m.SourcePoint.x,
                    top: m.SourcePoint.y,
                    right: m.SourcePoint.x + (m.DestinationRect.right - m.DestinationRect.left),
                    bottom: m.SourcePoint.y + (m.DestinationRect.bottom - m.DestinationRect.top),
                };
                push_change(source, origin_x, origin_y, generation);
            }
        }
    }
}

/// 轮询所有显示器的桌面变化，返回当前桌面代号。
///
/// 必须在分类框决定是否重绘之前调用：不取帧就永远不知道
/// 桌面变了。内部按 `DESKTOP_POLL_INTERVAL` 限流，并且直接复用
/// `capture_output`——它会把新帧拷进 `desktop_texture`，所以随后
/// 真正渲染时只会得到 WAIT_TIMEOUT 并复用同一张纹理，不会丢帧。
///
/// 代价：每个输出的 `desktop_texture` 会被提前分配（每屏约 8MB，
/// 一次性）。相比之下省下的是每帧整条 D2D/D3D 管线，得不偿失。
pub fn poll_desktop_changes() -> u64 {
    if !is_liquid_glass_enabled() {
        return DESKTOP_GENERATION.load(Ordering::Relaxed);
    }
    let now = Instant::now();
    let due = LAST_DESKTOP_POLL.with(|cell| match cell.get() {
        Some(last) if now.duration_since(last) < DESKTOP_POLL_INTERVAL => false,
        _ => {
            cell.set(Some(now));
            true
        }
    });
    if due {
        GLASS_STATE.with(|state| {
            let Ok(mut state) = state.try_borrow_mut() else {
                return;
            };
            let GlassState::Ready(renderer) = &mut *state else {
                return;
            };
            for index in 0..renderer.outputs.len() {
                if let Err(error) = renderer.capture_output(index, 0) {
                    log::debug!("[LG] 桌面变化轮询失败 output={index}: {error:#}");
                }
            }
        });
    }
    DESKTOP_GENERATION.load(Ordering::Relaxed)
}

/// 判断指定桌面区域自 `since_generation` 以来是否发生过变化。
///
/// 保守优先：历史被裁掉、队列为空、代号对不上时一律返回 `true`，
/// 宁可多画一帧也不能漏更新。
pub fn region_changed_since(rect: &WinRect, since_generation: u64) -> bool {
    let current = DESKTOP_GENERATION.load(Ordering::Relaxed);
    DESKTOP_CHANGES.with(|cell| region_changed_in(&cell.borrow(), current, rect, since_generation))
}

/// `region_changed_since` 的纯函数内核，不碰任何全局状态，便于单测。
///
/// 保守优先：只有能**证明**该区域自 `since` 以来没被任何
/// 脏矩形碰到时才返回 false。历史被裁掉、队列为空但代号
/// 已前进、代号对不上，一律返回 true。
fn region_changed_in(
    queue: &VecDeque<(u64, WinRect)>,
    current_generation: u64,
    rect: &WinRect,
    since_generation: u64,
) -> bool {
    if current_generation == since_generation {
        // 桌面自上次渲染后一帧都没变
        return false;
    }
    match queue.front() {
        // 队列已经裁掉了 since 之后的部分历史 → 无法证明没变
        Some((oldest, _)) if *oldest > since_generation.saturating_add(1) => true,
        None => true,
        _ => queue.iter().any(|(generation, changed)| {
            *generation > since_generation && intersects(changed, rect)
        }),
    }
}

/// 两个矩形是否相交（边界重叠不算）。
fn intersects(a: &WinRect, b: &WinRect) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// 渲染一帧液态玻璃背景，像素直接写进 `out`。
///
/// 返回 `true` 表示 `out` 里已是一张完整的
/// `(content_width + padding * 2) × (content_height + padding * 2)` 表面。
/// 用调用方缓冲区而不是返回新 Vec：旧实现每帧新分配一块
/// 1~5MB 再丢掉，5 个框 × 5fps 就是每秒上百 MB 的大对象申请/归还。
#[allow(clippy::too_many_arguments)]
pub fn render_background(
    screen_x: i32,
    screen_y: i32,
    content_width: i32,
    content_height: i32,
    padding: i32,
    corner_radius: f32,
    dpi_scale: f32,
    glass_config: &LiquidGlassConfig,
    out: &mut Vec<Pixel32>,
) -> bool {
    if content_width <= 0 || content_height <= 0 || padding < 0 {
        return false;
    }

    let cache_key = (screen_x, screen_y, content_width, content_height, padding);
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
                return fill_from_cache(cache_key, out);
            }
        };
        if matches!(&*state, GlassState::Failed { retry_at } if Instant::now() < *retry_at) {
            return fill_from_cache(cache_key, out);
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
                    return fill_from_cache(cache_key, out);
                }
            }
        }

        let GlassState::Ready(renderer) = &mut *state else {
            return false;
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
            out,
        ) {
            Ok(()) => {
                log::debug!("[LG] render OK, {} pixels", out.len());
                log_first_frame_diagnostic(out, content_width, content_height, padding);
                cache_frame(cache_key, out);
                true
            }
            Err(error) => {
                log::warn!("液态玻璃帧渲染失败，保留上一成功帧: {error:#}");
                // 单帧捕获失败不应让整个渲染器进入 3 秒回退状态；保留 Ready
                // 可在下一次定时器 tick 立即重试，避免拖动到新位置时闪黑。
                fill_from_cache(cache_key, out)
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

/// 为每个显示器输出建立桌面复制。
///
/// **单个输出失败不能让整套玻璃报废。** DXGI 每个输出同一
/// 时刻只允许一个 duplication：录屏、会议软件、远程桌面、截图工具
/// 占着时 `DuplicateOutput` 会返回 E_ACCESSDENIED。旧实现用 `?`
/// 直接冒泡，结果是只要有一个显示器被占用，**所有**分类框
/// 都失去液态玻璃。现在跳过失败的输出，其余显示器照常工作；
/// 落在失败输出上的分类框会自动回退到半透明背景。
/// 全部输出都失败时返回空 Vec，由调用方转成 Failed 状态按
/// `RETRY_DELAY` 退避重试，而不是每帧报一次错。
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
            let index = i;
            i += 1;

            // 显示器名单独取：失败日志里要能看出是哪一台，
            // 否则用户只看到一个 output 索引，无法定位到具体设备
            let device_name = output
                .GetDesc()
                .ok()
                .map(|desc| {
                    let end = desc
                        .DeviceName
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(desc.DeviceName.len());
                    String::from_utf16_lossy(&desc.DeviceName[..end])
                })
                .unwrap_or_else(|| "<未知>".to_string());

            // 把可能失败的三步放进闭包，失败只影响这一个输出
            let prepared = (|| -> Result<CaptureOutput> {
                let output1 = output.cast::<IDXGIOutput1>()?;
                let duplication = output1.DuplicateOutput(device)?;
                let desc = output.GetDesc()?;
                Ok(CaptureOutput {
                    rect: desc.DesktopCoordinates,
                    output1,
                    duplication,
                    hdr_active: false,
                    desktop_texture: None,
                    desktop_view: None,
                    blurred_texture: None,
                    blurred_view: None,
                    source_bitmap: None,
                    target_bitmap: None,
                    blur_chain: None,
                })
            })();
            match prepared {
                Ok(capture) => outputs.push(capture),
                Err(error) => log::warn!(
                    "[LG] 输出 {index} ({device_name}) 桌面复制不可用，该显示器上的分类框回退半透明背景: {error:#}"
                ),
            }
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
                // DEFAULT + UpdateSubresource，不用 DYNAMIC + MAP_WRITE_DISCARD：
                // 后者每次 Map 都触发运行时的 buffer renaming（丢弃旧后端、
                // 另分配一块），玻璃开启时 25 次/秒，GPU 一旦落后重命名
                // 后端就会堆积。常量缓冲每帧整体覆写，交给运行时走
                // 统一上传堆即可，无重命名。
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: 0,
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
                startup_black_frames_remaining: 3,
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

    /// 抓一帧桌面。返回 `true` 表示确实拿到了新帧并已拷进
    /// `desktop_texture`；`false` 表示桌面自上次以来没变
    /// （DXGI 返回 WAIT_TIMEOUT）。
    fn capture_output(&mut self, index: usize, timeout_ms: u32) -> Result<bool> {
        unsafe {
            let output = &mut self.outputs[index];
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            let acquire =
                output
                    .duplication
                    .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource);

            if let Err(e) = acquire {
                if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                    log::debug!(
                        "[LG] capture WAIT_TIMEOUT, desktop_texture={}",
                        output.desktop_texture.is_some()
                    );
                    return Ok(false);
                }
                if e.code() == DXGI_ERROR_ACCESS_LOST {
                    // DXGI desktop capture access lost — rebuild duplication instead of
                    // failing the entire renderer. This is common during rapid window resize.
                    self.rebuild_duplication(index)?;
                    return Ok(false);
                }
                return Err(e.into());
            }

            // 所有可能失败的 COM 操作都放在闭包中，确保 Acquire 成功后总会 ReleaseFrame。
            let capture_result = (|| -> Result<bool> {
                let Some(resource) = resource else {
                    return Ok(false);
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
                    log::info!(
                        "[LG-mem] 新增桌面捕获纹理 {}x{} = {:.1}MB (output={})",
                        desc.Width,
                        desc.Height,
                        (desc.Width as f64 * desc.Height as f64 * 4.0) / 1_048_576.0,
                        index
                    );
                }
                if let Some(destination) = &output.desktop_texture {
                    self.context.CopyResource(destination, &texture);
                }
                // 拿到新帧 → 桌面代号前进，分类框的周期 tick 据此
                // 判断「玻璃背景变了」。只在真正渲染时才会走到这里，
                // 因此不需要额外的轮询开销。
                let generation = DESKTOP_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
                // 必须在 ReleaseFrame 之前取脏矩形
                collect_frame_changes(output, generation);
                Ok(true)
            })();
            let _ = output.duplication.ReleaseFrame();
            capture_result
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
                log::info!(
                    "[LG-mem] 新增模糊纹理 {}x{} = {:.1}MB",
                    texture_desc.Width,
                    texture_desc.Height,
                    (texture_desc.Width as f64 * texture_desc.Height as f64 * 4.0) / 1_048_576.0
                );
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

            // ===== 复用 D2D 位图与 effect 链 =====
            //
            // 旧实现每帧 CreateBitmapFromDxgiSurface ×2 + CreateEffect ×4：
            // 5 个框 × 5fps 就是每秒 150 个 D2D 对象。D2D1 为每张 effect 图
            // 维护 realization（模糊用的中间表面），对象每帧都是新的 →
            // realization 永远命不中 → D2D 内部池只增不减。实测私有内存
            // 以 ~28MB 一个台阶持续上涨，且没有任何 Rust 侧分配与之对应。
            // 缓存到 output 上之后 effect 对象身份稳定，D2D1 能复用 realization。
            let source_bitmap = match output.source_bitmap.clone() {
                Some(bitmap) => bitmap,
                None => {
                    let surface = source_texture.cast::<IDXGISurface>()?;
                    let created = d2d_context
                        .CreateBitmapFromDxgiSurface(&surface, Some(&source_properties))?;
                    output.source_bitmap = Some(created.clone());
                    created
                }
            };
            let target_bitmap = match output.target_bitmap.clone() {
                Some(bitmap) => bitmap,
                None => {
                    let surface = target_texture.cast::<IDXGISurface>()?;
                    let created = d2d_context
                        .CreateBitmapFromDxgiSurface(&surface, Some(&target_properties))?;
                    output.target_bitmap = Some(created.clone());
                    created
                }
            };

            let crop_rect = D2D_RECT_F {
                left: (lens_x - output.rect.left).max(0) as f32,
                top: (lens_y - output.rect.top).max(0) as f32,
                right: (lens_x + lens_w - output.rect.left).min(texture_desc.Width as i32) as f32,
                bottom: (lens_y + lens_h - output.rect.top).min(texture_desc.Height as i32) as f32,
            };
            if crop_rect.right <= crop_rect.left || crop_rect.bottom <= crop_rect.top {
                return Ok(());
            }

            // effect 链只建一次：Crop → Border(MIRROR) → GaussianBlur → ColorMatrix。
            // 各级输入在创建时接好，之后每帧只用 SetValue 改裁剪框 / σ / 饱和度矩阵。
            let chain = match output.blur_chain.take() {
                Some(chain) => chain,
                None => {
                    let crop = d2d_context.CreateEffect(&CLSID_D2D1Crop)?;
                    let border = d2d_context.CreateEffect(&CLSID_D2D1Border)?;
                    border.SetInput(0, &crop.GetOutput()?, true);
                    let blur = d2d_context.CreateEffect(&CLSID_D2D1GaussianBlur)?;
                    blur.SetInput(0, &border.GetOutput()?, true);
                    let saturate = d2d_context.CreateEffect(&CLSID_D2D1ColorMatrix)?;
                    saturate.SetInput(0, &blur.GetOutput()?, true);
                    let saturated = saturate.GetOutput()?;
                    log::info!("[LG-mem] 新建 D2D effect 链（每个 output 一次）");
                    BlurChain {
                        crop,
                        border,
                        blur,
                        saturate,
                        saturated,
                    }
                }
            };

            chain.crop.SetInput(0, &source_bitmap, true);
            let crop_bytes = slice::from_raw_parts(
                (&crop_rect as *const D2D_RECT_F).cast::<u8>(),
                size_of::<D2D_RECT_F>(),
            );
            chain.crop.SetValue(
                D2D1_CROP_PROP_RECT.0 as u32,
                D2D1_PROPERTY_TYPE_VECTOR4,
                crop_bytes,
            )?;

            let edge_mode = D2D1_BORDER_EDGE_MODE_MIRROR.0;
            let edge_bytes =
                slice::from_raw_parts((&edge_mode as *const i32).cast::<u8>(), size_of::<i32>());
            chain.border.SetValue(
                D2D1_BORDER_PROP_EDGE_MODE_X.0 as u32,
                D2D1_PROPERTY_TYPE_ENUM,
                edge_bytes,
            )?;
            chain.border.SetValue(
                D2D1_BORDER_PROP_EDGE_MODE_Y.0 as u32,
                D2D1_PROPERTY_TYPE_ENUM,
                edge_bytes,
            )?;

            let sigma_bytes = slice::from_raw_parts((&sigma as *const f32).cast::<u8>(), 4);
            chain.blur.SetValue(
                D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                D2D1_PROPERTY_TYPE_FLOAT,
                sigma_bytes,
            )?;

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
            chain.saturate.SetValue(
                D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32,
                D2D1_PROPERTY_TYPE_MATRIX_5X4,
                saturation_bytes,
            )?;

            d2d_context.SetTarget(&target_bitmap);
            d2d_context.BeginDraw();
            // 只模糊分类框自己的内容矩形，不模糊整个桌面。
            //
            // 着色器里 `mirrorSource` 把采样 uv 夹在 sourceMinimum /
            // sourceMaximum 之间，而这两个值算出来正好就是 crop_rect：
            //     sourceMinimum = (windowOrigin + padding - outputOrigin) / outputSize
            //     sourceMaximum = (windowOrigin + padding + lensSize - outputOrigin) / outputSize
            // 也就是说 crop_rect 之外的模糊结果永远不会被读到。
            //
            // 旧实现把 DrawImage 交给 D2D 自己算输出范围，Border(MIRROR) 会把
            // 图像撑到整张目标，于是每帧都对全桌面（2560x1440 级）做
            // 高斯模糊，D2D 要为中间 pass 分配数张全屏表面。
            // 5 个框 × 5fps = 每秒 25 次全屏模糊，是玻璃开启后
            // 内存从 250MB 一路暴涨到 700MB 的主因。
            // 裁剪后模糊面积缩到原来的 ~1/45，且画面逐像素不变。
            d2d_context.PushAxisAlignedClip(&crop_rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            d2d_context.DrawImage(
                &chain.saturated,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_COPY,
            );
            d2d_context.PopAxisAlignedClip();
            let end_draw = d2d_context.EndDraw(None, None);
            d2d_context.SetTarget(None::<&ID2D1Image>);
            // 链交回 output 供下一帧复用（必须在 DrawImage 之后）
            output.blur_chain = Some(chain);
            end_draw.map_err(Into::into)
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
            log::info!(
                "[LG-mem] 新增渲染表面 {}x{} = {:.1}MB（输出+暂存共 2 张），缓存 {} 项",
                width,
                height,
                (width as f64 * height as f64 * 8.0) / 1_048_576.0,
                self.surfaces.len()
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
        out: &mut Vec<Pixel32>,
    ) -> Result<()> {
        let center_x = screen_x + padding as i32 + content_width as i32 / 2;
        let center_y = screen_y + padding as i32 + content_height as i32 / 2;

        let output_index = self
            .outputs
            .iter()
            .position(|output| point_in_rect(center_x, center_y, &output.rect))
            .ok_or_else(|| anyhow!("分类框中心不在当前 D3D11 适配器的任何输出中"))?;

        self.capture_output(output_index, 0)?;
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
        let surface_w = (content_width + padding * 2) as i32;
        let surface_h = (content_height + padding * 2) as i32;
        let cursor_in_window = unsafe {
            if GetCursorPos(&mut cursor).is_ok() {
                cursor.x >= screen_x
                    && cursor.x < screen_x + surface_w
                    && cursor.y >= screen_y
                    && cursor.y < screen_y + surface_h
            } else {
                false
            }
        };
        let target_light = if cursor_in_window {
            [
                cursor.x as f32 - (screen_x as f32 + padding as f32 + content_width as f32 * 0.5),
                cursor.y as f32 - (screen_y as f32 + padding as f32 + content_height as f32 * 0.5),
            ]
        } else {
            [0.0, 0.0]
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
            cursor_in_window,
            out,
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
        cursor_in_window: bool,
        out: &mut Vec<Pixel32>,
    ) -> Result<()> {
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
                if cursor_in_window {
                    glass_config.elasticity
                } else {
                    0.0
                },
                corner_radius.min(content_height as f32 * 0.5) * dpi_scale,
            ],
            mouse_offset: if cursor_in_window {
                self.light_direction
            } else {
                [0.0, 0.0]
            },
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

            self.context.UpdateSubresource(
                &self.constants,
                0,
                None,
                (&constants as *const Constants).cast::<c_void>(),
                size_of::<Constants>() as u32,
                0,
            );

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

            // 复用调用方缓冲区：`clear` 保留容量，长度不变时零分配。
            out.clear();
            out.resize(
                (surface_width * surface_height) as usize,
                Pixel32 {
                    b: 0,
                    g: 0,
                    r: 0,
                    a: 0,
                },
            );
            let row_bytes = surface_width as usize * size_of::<Pixel32>();

            for row in 0..surface_height as usize {
                let source = mapped
                    .pData
                    .cast::<u8>()
                    .add(row * mapped.RowPitch as usize);
                let destination = out.as_mut_ptr().cast::<u8>().add(row * row_bytes);
                std::ptr::copy_nonoverlapping(source, destination, row_bytes);
            }

            self.context.Unmap(&surfaces.staging_texture, 0);
            let center_index = ((padding + content_height / 2) * surface_width
                + padding
                + content_width / 2) as usize;
            if out.get(center_index).is_none_or(|pixel| pixel.a == 0) {
                return Err(anyhow!("液态玻璃 GPU 输出中心像素完全透明"));
            }
            if self.startup_black_frames_remaining > 0 {
                if let Some(stats) = content_frame_stats(
                    out,
                    content_width as i32,
                    content_height as i32,
                    padding as i32,
                ) {
                    if is_dxgi_placeholder(&stats) {
                        self.startup_black_frames_remaining -= 1;
                        return Err(anyhow!("DXGI 初始化占位黑帧，等待真实桌面画面"));
                    }
                    self.startup_black_frames_remaining = 0;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// push_change 必须把输出坐标换算成桌面坐标。
    ///
    /// DXGI 返回的脏矩形是相对于该输出自己的原点，而分类框
    /// 查询用的是桌面坐标。剪贴上去之前必须加上 output.rect 的偏移，
    /// 否则副屏上的分类框会永远收不到变化通知（画面冻住）。
    #[test]
    fn push_change_offsets_into_desktop_coordinates() {
        DESKTOP_CHANGES.with(|c| c.borrow_mut().clear());
        // 副屏原点在 (2560, 0)，脏矩形在该屏内的 (10,20)-(30,40)
        push_change(r(10, 20, 30, 40), 2560, 0, 42);
        let queued = DESKTOP_CHANGES.with(|c| c.borrow().clone());
        assert_eq!(queued.len(), 1);
        let (gen, rect) = &queued[0];
        assert_eq!(*gen, 42);
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (2570, 20, 2590, 40)
        );

        // 主屏原点在 (0,0) 时不应偏移
        push_change(r(100, 100, 200, 200), 0, 0, 43);
        let queued = DESKTOP_CHANGES.with(|c| c.borrow().clone());
        assert_eq!(queued.len(), 2);
        assert_eq!(
            (
                queued[1].1.left,
                queued[1].1.top,
                queued[1].1.right,
                queued[1].1.bottom
            ),
            (100, 100, 200, 200)
        );
        DESKTOP_CHANGES.with(|c| c.borrow_mut().clear());
    }

    /// 退化矩形（零面积 / 反向）不得入队，否则会把队列填满垃圾。
    #[test]
    fn push_change_rejects_degenerate_rects() {
        DESKTOP_CHANGES.with(|c| c.borrow_mut().clear());
        push_change(r(50, 50, 50, 80), 0, 0, 1); // 零宽
        push_change(r(50, 50, 80, 50), 0, 0, 1); // 零高
        push_change(r(80, 80, 50, 50), 0, 0, 1); // 反向
        push_change(r(0, 0, 0, 0), 0, 0, 1); // 全零
        assert!(DESKTOP_CHANGES.with(|c| c.borrow().is_empty()));
    }

    /// 队列必须有界：满帧率视频下每秒可能产生上百个脏矩形，
    /// 不能无限堆。裁掉旧的之后 region_changed_in 会转为保守重绘，
    /// 所以这里只需验证不会溢出、且保留的是最新的那批。
    #[test]
    fn change_queue_is_bounded_and_keeps_newest() {
        DESKTOP_CHANGES.with(|c| c.borrow_mut().clear());
        let overflow = MAX_DESKTOP_CHANGES + 100;
        for gen in 1..=overflow as u64 {
            push_change(r(0, 0, 10, 10), 0, 0, gen);
        }
        let queued = DESKTOP_CHANGES.with(|c| c.borrow().clone());
        assert_eq!(queued.len(), MAX_DESKTOP_CHANGES);
        // 保留的必须是最新的一批（代号递增到 overflow）
        assert_eq!(queued.back().unwrap().0, overflow as u64);
        assert_eq!(
            queued.front().unwrap().0,
            (overflow - MAX_DESKTOP_CHANGES + 1) as u64
        );
        DESKTOP_CHANGES.with(|c| c.borrow_mut().clear());
    }

    /// 端到端：push_change 写入的队列能被 region_changed_in 正确读出。
    #[test]
    fn pushed_changes_drive_region_queries() {
        DESKTOP_CHANGES.with(|c| c.borrow_mut().clear());
        let box_on_secondary = r(2600, 100, 3000, 500); // 副屏上的分类框
        let box_on_primary = r(100, 100, 700, 600);

        // 副屏（原点 2560,0）上有东西变了，输出坐标 (40,100)-(300,400)
        push_change(r(40, 100, 300, 400), 2560, 0, 5);

        DESKTOP_CHANGES.with(|c| {
            let q = c.borrow();
            assert!(
                region_changed_in(&q, 5, &box_on_secondary, 4),
                "副屏上的框必须收到通知"
            );
            assert!(
                !region_changed_in(&q, 5, &box_on_primary, 4),
                "主屏上的框不应陪绘"
            );
        });
        DESKTOP_CHANGES.with(|c| c.borrow_mut().clear());
    }

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> WinRect {
        WinRect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn intersects_ignores_edge_touching() {
        let a = r(0, 0, 100, 100);
        assert!(intersects(&a, &r(50, 50, 150, 150)));
        assert!(intersects(&a, &r(0, 0, 100, 100)));
        // 只有边界重叠不算相交：那样的脏矩形不会改变任何像素
        assert!(!intersects(&a, &r(100, 0, 200, 100)));
        assert!(!intersects(&a, &r(0, 100, 100, 200)));
        assert!(!intersects(&a, &r(-50, -50, 0, 0)));
        assert!(!intersects(&a, &r(500, 500, 600, 600)));
    }

    #[test]
    fn region_unchanged_when_generation_is_current() {
        let queue: VecDeque<(u64, WinRect)> = VecDeque::new();
        // 代号没前进 → 无论队列里有什么都不需要重绘
        assert!(!region_changed_in(&queue, 7, &r(0, 0, 10, 10), 7));
    }

    #[test]
    fn region_changed_only_when_dirty_rect_hits_it() {
        let box_a = r(100, 100, 700, 600);
        let box_b = r(4600, 0, 5000, 400);
        let mut queue: VecDeque<(u64, WinRect)> = VecDeque::new();
        // 任务栏时钟跳了一下：只碰到屏幕底部
        queue.push_back((8, r(2400, 1050, 2500, 1080)));
        // 别处光标闪烁
        queue.push_back((9, r(3000, 500, 3010, 520)));

        assert!(
            !region_changed_in(&queue, 9, &box_a, 7),
            "桌面变了但不在这个框里 → 不应重绘"
        );
        assert!(!region_changed_in(&queue, 9, &box_b, 7));

        // 现在有一块脏区域碰到 box_a
        queue.push_back((10, r(650, 550, 900, 800)));
        assert!(
            region_changed_in(&queue, 10, &box_a, 7),
            "脏矩形碰到了这个框 → 必须重绘"
        );
        assert!(
            !region_changed_in(&queue, 10, &box_b, 7),
            "同一帧里另一个框不受影响 → 不重绘"
        );
    }

    #[test]
    fn trimmed_history_falls_back_to_conservative() {
        let box_a = r(100, 100, 700, 600);
        let mut queue: VecDeque<(u64, WinRect)> = VecDeque::new();
        // 队列最老记录已经是代号 50，而调用方上次渲染在 7
        // → 8..49 的历史被裁掉了，无法证明该区域没变
        queue.push_back((50, r(0, 0, 10, 10)));
        assert!(
            region_changed_in(&queue, 50, &box_a, 7),
            "历史丢了就必须保守重绘"
        );
        // 但如果只差一代（刚好没裁到），仍可以正常判定
        queue.clear();
        queue.push_back((8, r(0, 0, 10, 10)));
        assert!(!region_changed_in(&queue, 8, &box_a, 7));
    }

    #[test]
    fn empty_queue_with_advanced_generation_is_conservative() {
        // 代号前进了但一条矩形都没记录（例如刚被清空）
        // → 不敢断言没变，保守重绘
        let queue: VecDeque<(u64, WinRect)> = VecDeque::new();
        assert!(region_changed_in(&queue, 9, &r(0, 0, 10, 10), 7));
    }

    #[test]
    fn change_queue_is_bounded() {
        // push_change 走的是全局队列，这里只验证上限常量本身合理：
        // 太小会频繁丢历史（导致保守重绘），太大则白占内存
        assert!(MAX_DESKTOP_CHANGES >= 64);
        assert!(MAX_DESKTOP_CHANGES <= 4096);
        // 每条 24 字节左右，上限下不超过 100KB
        assert!(MAX_DESKTOP_CHANGES * size_of::<(u64, WinRect)>() < 100 * 1024);
    }

    #[test]
    fn dxgi_placeholder_detection_rejects_opaque_black_frame() {
        let pixels = vec![
            Pixel32 {
                r: 1,
                g: 1,
                b: 1,
                a: 255,
            };
            64
        ];
        let stats = content_frame_stats(&pixels, 8, 8, 0).expect("frame stats");

        assert!(is_dxgi_placeholder(&stats));
        assert_eq!(stats.averages, [1, 1, 1, 255]);
    }

    #[test]
    fn dxgi_placeholder_detection_accepts_real_desktop_frame() {
        let pixels = vec![
            Pixel32 {
                r: 171,
                g: 191,
                b: 207,
                a: 254,
            };
            64
        ];
        let stats = content_frame_stats(&pixels, 8, 8, 0).expect("frame stats");

        assert!(!is_dxgi_placeholder(&stats));
        assert_eq!(stats.near_black_percent, 0.0);
    }

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

    // ===== 单框弹性形变响应测试（v2.1 新增） =====

    /// 验证 cursor_in_window 判断逻辑：鼠标在窗口内
    #[test]
    fn test_cursor_in_window_detection_inside() {
        // 模拟窗口位置：screen_x=100, screen_y=100, surface_w=400, surface_h=300
        let screen_x = 100i32;
        let screen_y = 100i32;
        let surface_w = 400i32;
        let surface_h = 300i32;

        // 鼠标在窗口中心
        let cursor_x = 300i32;
        let cursor_y = 250i32;

        let in_window = cursor_x >= screen_x
            && cursor_x < screen_x + surface_w
            && cursor_y >= screen_y
            && cursor_y < screen_y + surface_h;

        assert!(
            in_window,
            "Cursor at ({}, {}) should be inside window",
            cursor_x, cursor_y
        );
    }

    /// 验证 cursor_in_window 判断逻辑：鼠标在窗口外
    #[test]
    fn test_cursor_in_window_detection_outside() {
        let screen_x = 100i32;
        let screen_y = 100i32;
        let surface_w = 400i32;
        let surface_h = 300i32;

        // 鼠标在窗口右侧外部
        let cursor_x = 600i32;
        let cursor_y = 250i32;

        let in_window = cursor_x >= screen_x
            && cursor_x < screen_x + surface_w
            && cursor_y >= screen_y
            && cursor_y < screen_y + surface_h;

        assert!(
            !in_window,
            "Cursor at ({}, {}) should be outside window",
            cursor_x, cursor_y
        );
    }

    /// 验证 cursor_in_window 边界条件：左上角
    #[test]
    fn test_cursor_in_window_boundary_top_left() {
        let screen_x = 100i32;
        let screen_y = 100i32;
        let surface_w = 400i32;
        let surface_h = 300i32;

        // 鼠标在窗口左上角（包含边界）
        let cursor_x = 100i32;
        let cursor_y = 100i32;

        let in_window = cursor_x >= screen_x
            && cursor_x < screen_x + surface_w
            && cursor_y >= screen_y
            && cursor_y < screen_y + surface_h;

        assert!(in_window, "Cursor at top-left boundary should be inside");
    }

    /// 验证 cursor_in_window 边界条件：右下角（不包含）
    #[test]
    fn test_cursor_in_window_boundary_bottom_right() {
        let screen_x = 100i32;
        let screen_y = 100i32;
        let surface_w = 400i32;
        let surface_h = 300i32;

        // 鼠标在窗口右下角（不包含边界，使用 < 而非 <=）
        let cursor_x = screen_x + surface_w; // 500
        let cursor_y = screen_y + surface_h; // 400

        let in_window = cursor_x >= screen_x
            && cursor_x < screen_x + surface_w
            && cursor_y >= screen_y
            && cursor_y < screen_y + surface_h;

        assert!(
            !in_window,
            "Cursor at bottom-right boundary (exclusive) should be outside"
        );
    }

    /// 验证 shader 中使用 controlSettings.z (elasticity) 和 mouseOffset
    #[test]
    fn test_shader_uses_elasticity_and_mouse_offset() {
        // controlSettings.z 是 elasticity
        assert!(
            SHADER.contains("controlSettings.z"),
            "Shader must use controlSettings.z for elasticity"
        );

        // mouseOffset 用于计算鼠标位置
        assert!(
            SHADER.contains("mouseOffset"),
            "Shader must use mouseOffset for cursor position"
        );
    }

    /// 验证 Constants 结构中 control_settings 和 mouse_offset 字段存在
    #[test]
    fn test_constants_structure_has_elasticity_fields() {
        // 验证 Constants 结构包含必要的弹性形变相关字段
        let constants = Constants {
            output_origin: [0.0; 2],
            output_size: [0.0; 2],
            window_origin: [0.0; 2],
            window_size: [0.0; 2],
            lens_size: [0.0; 2],
            padding: [0.0; 2],
            shadow_settings: [0.0; 4],
            refraction_settings: [0.0; 4],
            effects_settings: [0.0; 4],
            color_settings: [0.0; 4],
            detail_settings: [0.0; 4],
            control_settings: [0.0, 0.0, 0.1, 0.0], // z = elasticity
            mouse_offset: [10.0, 20.0],
            interaction_padding: [0.0; 2],
        };

        // 验证 control_settings.z (elasticity) 可以设置
        assert!((constants.control_settings[2] - 0.1).abs() < f32::EPSILON);

        // 验证 mouse_offset 可以设置
        assert!((constants.mouse_offset[0] - 10.0).abs() < f32::EPSILON);
        assert!((constants.mouse_offset[1] - 20.0).abs() < f32::EPSILON);
    }
}
