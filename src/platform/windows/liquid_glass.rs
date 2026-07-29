//! 分类框液态玻璃 GPU 渲染器。
//!
//! 使用 DXGI Desktop Duplication 共享捕获显示器画面，通过 D3D11 两遍可分离
//! 高斯模糊和液态玻璃像素着色器生成预乘 alpha BGRA 位图。输出继续交给现有
//! Direct2D + UpdateLayeredWindow 管线合成图标和文字。

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::mem::size_of;
use std::slice;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HMODULE, RECT as WinRect};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

use super::ffi_types::Pixel32;

const RETRY_DELAY: Duration = Duration::from_secs(3);
const MAX_SURFACE_CACHE_ENTRIES: usize = 8;
const BLUR_SCALE: f32 = 2.8;
const REFRACTION_STRENGTH: f32 = 7.0;
const GLASS_ALPHA: f32 = 0.88;

const SHADER: &str = r#"
cbuffer GlassConstants : register(b0) {
    float4 sourceRect;
    float4 targetInfo;
    float4 glassInfo;
    float4 colorInfo;
};

Texture2D sourceTexture : register(t0);
SamplerState linearSampler : register(s0);

struct VertexOutput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

VertexOutput VSMain(uint vertexId : SV_VertexID) {
    VertexOutput output;
    float2 position = float2((vertexId << 1) & 2, vertexId & 2);
    output.uv = position;
    output.position = float4(position * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return output;
}

float4 gaussianHorizontal(float2 desktopUv) {
    float2 offset = float2(targetInfo.z * glassInfo.y, 0.0);
    float4 color = sourceTexture.Sample(linearSampler, desktopUv) * 0.2270270270;
    color += sourceTexture.Sample(linearSampler, desktopUv + offset * 1.3846153846) * 0.3162162162;
    color += sourceTexture.Sample(linearSampler, desktopUv - offset * 1.3846153846) * 0.3162162162;
    color += sourceTexture.Sample(linearSampler, desktopUv + offset * 3.2307692308) * 0.0702702703;
    color += sourceTexture.Sample(linearSampler, desktopUv - offset * 3.2307692308) * 0.0702702703;
    return color;
}

float4 PSBlurHorizontal(VertexOutput input) : SV_TARGET {
    float2 desktopUv = sourceRect.xy + input.uv * sourceRect.zw;
    return gaussianHorizontal(desktopUv);
}

float roundedRectSdf(float2 position, float2 halfSize, float radius) {
    float2 distance = abs(position) - halfSize + radius;
    return min(max(distance.x, distance.y), 0.0) + length(max(distance, 0.0)) - radius;
}

float4 gaussianVertical(float2 uv) {
    float2 offset = float2(0.0, targetInfo.w * glassInfo.y);
    float4 color = sourceTexture.Sample(linearSampler, uv) * 0.2270270270;
    color += sourceTexture.Sample(linearSampler, uv + offset * 1.3846153846) * 0.3162162162;
    color += sourceTexture.Sample(linearSampler, uv - offset * 1.3846153846) * 0.3162162162;
    color += sourceTexture.Sample(linearSampler, uv + offset * 3.2307692308) * 0.0702702703;
    color += sourceTexture.Sample(linearSampler, uv - offset * 3.2307692308) * 0.0702702703;
    return color;
}

float4 PSGlass(VertexOutput input) : SV_TARGET {
    float2 size = targetInfo.xy;
    float2 localPosition = input.uv * size - size * 0.5;
    float radius = min(glassInfo.x, min(size.x, size.y) * 0.5);
    float distance = roundedRectSdf(localPosition, size * 0.5, radius);
    float aa = max(fwidth(distance), 0.75);
    float mask = smoothstep(aa, -aa, distance);

    float edge = saturate(1.0 - smoothstep(0.0, glassInfo.z, -distance));
    float2 radial = normalize(localPosition + float2(0.001, 0.001));
    float2 refractedUv = input.uv - radial * edge * edge * colorInfo.x / size;

    float3 color = gaussianVertical(refractedUv).rgb;
    float chroma = edge * edge * colorInfo.y;
    color.r = gaussianVertical(refractedUv + radial * chroma / size).r;
    color.b = gaussianVertical(refractedUv - radial * chroma / size).b;
    color = saturate((color - 0.5) * colorInfo.z + 0.5);

    float topLight = edge * saturate(0.55 - input.uv.y) * 0.16;
    float lowerShade = edge * saturate(input.uv.y - 0.35) * 0.10;
    color = saturate(color + topLight - lowerShade + colorInfo.w);

    float alpha = mask * glassInfo.w;
    return float4(color * alpha, alpha);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GlassConstants {
    source_rect: [f32; 4],
    target_info: [f32; 4],
    glass_info: [f32; 4],
    color_info: [f32; 4],
}

struct CaptureOutput {
    rect: WinRect,
    duplication: IDXGIOutputDuplication,
    desktop_texture: Option<ID3D11Texture2D>,
    desktop_view: Option<ID3D11ShaderResourceView>,
}

struct RenderSurfaces {
    width: u32,
    height: u32,
    blur_texture: ID3D11Texture2D,
    blur_view: ID3D11ShaderResourceView,
    blur_target: ID3D11RenderTargetView,
    output_texture: ID3D11Texture2D,
    output_target: ID3D11RenderTargetView,
    staging_texture: ID3D11Texture2D,
}

struct GlassRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vertex_shader: ID3D11VertexShader,
    horizontal_shader: ID3D11PixelShader,
    glass_shader: ID3D11PixelShader,
    constants: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    outputs: Vec<CaptureOutput>,
    surfaces: HashMap<(u32, u32), RenderSurfaces>,
}

enum GlassState {
    Uninitialized,
    Ready(GlassRenderer),
    Failed { retry_at: Instant },
}

thread_local! {
    static GLASS_STATE: RefCell<GlassState> = const { RefCell::new(GlassState::Uninitialized) };
}

/// 生成指定分类框的实时液态玻璃背景。
///
/// 返回预乘 alpha BGRA 像素；任何 GPU 或捕获错误都会返回 `None`，调用方应绘制
/// 原有半透明背景完成优雅降级。
pub fn render_background(
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
    corner_radius: f32,
) -> Option<Vec<Pixel32>> {
    if width <= 0 || height <= 0 {
        return None;
    }

    GLASS_STATE.with(|state| {
        let mut state = state.borrow_mut();
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
            width as u32,
            height as u32,
            corner_radius,
        ) {
            Ok(pixels) => Some(pixels),
            Err(error) => {
                log::warn!("液态玻璃渲染失败，暂时使用半透明背景: {error:#}");
                *state = GlassState::Failed {
                    retry_at: Instant::now() + RETRY_DELAY,
                };
                None
            }
        }
    })
}

impl GlassRenderer {
    fn new() -> Result<Self> {
        unsafe {
            let (device, context) = create_device()?;
            let vertex_blob = compile_shader(b"VSMain\0", b"vs_5_0\0")?;
            let horizontal_blob = compile_shader(b"PSBlurHorizontal\0", b"ps_5_0\0")?;
            let glass_blob = compile_shader(b"PSGlass\0", b"ps_5_0\0")?;

            let mut vertex_shader = None;
            device.CreateVertexShader(blob_bytes(&vertex_blob), None, Some(&mut vertex_shader))?;
            let mut horizontal_shader = None;
            device.CreatePixelShader(
                blob_bytes(&horizontal_blob),
                None,
                Some(&mut horizontal_shader),
            )?;
            let mut glass_shader = None;
            device.CreatePixelShader(blob_bytes(&glass_blob), None, Some(&mut glass_shader))?;

            let buffer_desc = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<GlassConstants>() as u32,
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

            let outputs = create_outputs(&device)?;
            if outputs.is_empty() {
                return Err(anyhow!("未找到可用的 DXGI 显示输出"));
            }

            Ok(Self {
                device,
                context,
                vertex_shader: vertex_shader.context("创建顶点着色器未返回对象")?,
                horizontal_shader: horizontal_shader.context("创建水平模糊着色器未返回对象")?,
                glass_shader: glass_shader.context("创建玻璃着色器未返回对象")?,
                constants: constants.context("创建常量缓冲区未返回对象")?,
                sampler: sampler.context("创建采样器未返回对象")?,
                outputs,
                surfaces: HashMap::new(),
            })
        }
    }

    fn render(
        &mut self,
        screen_x: i32,
        screen_y: i32,
        width: u32,
        height: u32,
        corner_radius: f32,
    ) -> Result<Vec<Pixel32>> {
        let center_x = screen_x + width as i32 / 2;
        let center_y = screen_y + height as i32 / 2;
        let output_index = self
            .outputs
            .iter()
            .position(|output| point_in_rect(center_x, center_y, &output.rect))
            .ok_or_else(|| anyhow!("分类框中心不在当前 D3D11 适配器的任何输出中"))?;

        self.capture_output(output_index)?;

        let output = &self.outputs[output_index];
        let desktop_view = output
            .desktop_view
            .as_ref()
            .context("尚未取得桌面捕获帧")?
            .clone();
        let output_width = (output.rect.right - output.rect.left).max(1) as f32;
        let output_height = (output.rect.bottom - output.rect.top).max(1) as f32;
        self.render_view(
            &desktop_view,
            [
                (screen_x - output.rect.left) as f32 / output_width,
                (screen_y - output.rect.top) as f32 / output_height,
                width as f32 / output_width,
                height as f32 / output_height,
            ],
            [output_width, output_height],
            width,
            height,
            corner_radius,
        )
    }

    fn render_view(
        &mut self,
        source_view: &ID3D11ShaderResourceView,
        source_rect: [f32; 4],
        source_size: [f32; 2],
        width: u32,
        height: u32,
        corner_radius: f32,
    ) -> Result<Vec<Pixel32>> {
        self.ensure_surfaces(width, height)?;
        let constants = GlassConstants {
            source_rect,
            target_info: [
                width as f32,
                height as f32,
                1.0 / source_size[0].max(1.0),
                1.0 / height.max(1) as f32,
            ],
            glass_info: [corner_radius, BLUR_SCALE, 26.0, GLASS_ALPHA],
            color_info: [REFRACTION_STRENGTH, 1.2, 1.04, 0.015],
        };

        unsafe {
            self.update_constants(constants)?;
            let surfaces = self
                .surfaces
                .get(&(width, height))
                .context("玻璃渲染表面未创建")?;
            self.draw_pass(
                source_view,
                &surfaces.blur_target,
                &self.horizontal_shader,
                width,
                height,
            );
            self.draw_pass(
                &surfaces.blur_view,
                &surfaces.output_target,
                &self.glass_shader,
                width,
                height,
            );
            self.context.PSSetShaderResources(0, Some(&[None]));
            self.context
                .CopyResource(&surfaces.staging_texture, &surfaces.output_texture);
            self.read_pixels(&surfaces.staging_texture, width, height)
        }
    }

    fn capture_output(&mut self, index: usize) -> Result<()> {
        let output = self.outputs.get_mut(index).context("DXGI 输出索引越界")?;
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        let timeout_ms = if output.desktop_view.is_some() {
            0
        } else {
            100
        };
        let acquire = unsafe {
            output
                .duplication
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        };

        if let Err(error) = acquire {
            if error.code() == DXGI_ERROR_WAIT_TIMEOUT && output.desktop_view.is_some() {
                return Ok(());
            }
            if error.code() == DXGI_ERROR_ACCESS_LOST {
                return Err(anyhow!("桌面捕获访问已丢失，需要重建设备"));
            }
            return Err(error).context("获取首个桌面捕获帧失败");
        }

        let result = (|| -> Result<()> {
            let frame = resource
                .context("Desktop Duplication 未返回帧资源")?
                .cast::<ID3D11Texture2D>()?;
            let mut frame_desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { frame.GetDesc(&mut frame_desc) };

            let recreate = output.desktop_texture.as_ref().is_none_or(|texture| {
                let mut current_desc = D3D11_TEXTURE2D_DESC::default();
                unsafe { texture.GetDesc(&mut current_desc) };
                current_desc.Width != frame_desc.Width
                    || current_desc.Height != frame_desc.Height
                    || current_desc.Format != frame_desc.Format
            });

            if recreate {
                frame_desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
                frame_desc.CPUAccessFlags = 0;
                frame_desc.Usage = D3D11_USAGE_DEFAULT;
                frame_desc.MiscFlags = 0;
                let mut texture = None;
                unsafe {
                    self.device
                        .CreateTexture2D(&frame_desc, None, Some(&mut texture))?;
                }
                let texture = texture.context("创建桌面纹理未返回对象")?;
                let mut view = None;
                unsafe {
                    self.device
                        .CreateShaderResourceView(&texture, None, Some(&mut view))?;
                }
                output.desktop_texture = Some(texture);
                output.desktop_view = view;
            }

            if let Some(texture) = &output.desktop_texture {
                unsafe { self.context.CopyResource(texture, &frame) };
            }
            Ok(())
        })();

        let _ = unsafe { output.duplication.ReleaseFrame() };
        result
    }

    fn ensure_surfaces(&mut self, width: u32, height: u32) -> Result<()> {
        let key = (width, height);
        if self.surfaces.contains_key(&key) {
            return Ok(());
        }

        if self.surfaces.len() >= MAX_SURFACE_CACHE_ENTRIES {
            if let Some(oldest_key) = self.surfaces.keys().next().copied() {
                self.surfaces.remove(&oldest_key);
            }
        }

        self.surfaces.insert(key, unsafe {
            create_surfaces(&self.device, width, height)?
        });
        Ok(())
    }

    unsafe fn update_constants(&self, constants: GlassConstants) -> Result<()> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        self.context.Map(
            &self.constants,
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            Some(&mut mapped),
        )?;
        mapped.pData.cast::<GlassConstants>().write(constants);
        self.context.Unmap(&self.constants, 0);
        Ok(())
    }

    unsafe fn draw_pass(
        &self,
        source: &ID3D11ShaderResourceView,
        target: &ID3D11RenderTargetView,
        shader: &ID3D11PixelShader,
        width: u32,
        height: u32,
    ) {
        self.context
            .OMSetRenderTargets(Some(&[Some(target.clone())]), None);
        self.context.ClearRenderTargetView(target, &[0.0; 4]);
        self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            Width: width as f32,
            Height: height as f32,
            MaxDepth: 1.0,
            ..Default::default()
        }]));
        self.context.IASetInputLayout(None);
        self.context
            .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.context.VSSetShader(&self.vertex_shader, None);
        self.context.PSSetShader(shader, None);
        self.context
            .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
        self.context
            .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
        self.context
            .PSSetShaderResources(0, Some(&[Some(source.clone())]));
        self.context.Draw(3, 0);
        self.context.PSSetShaderResources(0, Some(&[None]));
    }

    unsafe fn read_pixels(
        &self,
        staging: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<Vec<Pixel32>> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        self.context
            .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

        let mut pixels = vec![
            Pixel32 {
                b: 0,
                g: 0,
                r: 0,
                a: 0,
            };
            width as usize * height as usize
        ];
        let row_bytes = width as usize * size_of::<Pixel32>();
        for row in 0..height as usize {
            let source = mapped
                .pData
                .cast::<u8>()
                .add(row * mapped.RowPitch as usize);
            let destination = pixels.as_mut_ptr().cast::<u8>().add(row * row_bytes);
            std::ptr::copy_nonoverlapping(source, destination, row_bytes);
        }

        self.context.Unmap(staging, 0);
        Ok(pixels)
    }
}

unsafe fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        let mut context = None;
        let mut selected_level = D3D_FEATURE_LEVEL::default();
        let result = D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut selected_level),
            Some(&mut context),
        );
        if result.is_ok() {
            return Ok((
                device.context("D3D11 创建设备未返回对象")?,
                context.context("D3D11 创建设备上下文未返回对象")?,
            ));
        }
    }
    Err(anyhow!("硬件和 WARP D3D11 设备均创建失败"))
}

unsafe fn create_outputs(device: &ID3D11Device) -> Result<Vec<CaptureOutput>> {
    let dxgi_device: IDXGIDevice = device.cast()?;
    let adapter = dxgi_device.GetAdapter()?;
    let mut outputs = Vec::new();
    let mut index = 0;
    while let Ok(output) = adapter.EnumOutputs(index) {
        let desc = output.GetDesc()?;
        let output1: IDXGIOutput1 = output.cast()?;
        match output1.DuplicateOutput(device) {
            Ok(duplication) => outputs.push(CaptureOutput {
                rect: desc.DesktopCoordinates,
                duplication,
                desktop_texture: None,
                desktop_view: None,
            }),
            Err(error) => log::warn!(
                "创建显示输出 {} 的 Desktop Duplication 失败: {error}",
                index
            ),
        }
        index += 1;
    }
    Ok(outputs)
}

unsafe fn create_surfaces(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<RenderSurfaces> {
    let render_desc = D3D11_TEXTURE2D_DESC {
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
        BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
        ..Default::default()
    };

    let blur_texture = create_texture(device, &render_desc)?;
    let blur_view = create_shader_view(device, &blur_texture)?;
    let blur_target = create_render_target(device, &blur_texture)?;
    let output_texture = create_texture(device, &render_desc)?;
    let output_target = create_render_target(device, &output_texture)?;

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..render_desc
    };
    let staging_texture = create_texture(device, &staging_desc)?;

    Ok(RenderSurfaces {
        width,
        height,
        blur_texture,
        blur_view,
        blur_target,
        output_texture,
        output_target,
        staging_texture,
    })
}

unsafe fn create_texture(
    device: &ID3D11Device,
    desc: &D3D11_TEXTURE2D_DESC,
) -> Result<ID3D11Texture2D> {
    let mut texture = None;
    device.CreateTexture2D(desc, None, Some(&mut texture))?;
    texture.context("创建 D3D11 纹理未返回对象")
}

unsafe fn create_shader_view(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
) -> Result<ID3D11ShaderResourceView> {
    let mut view = None;
    device.CreateShaderResourceView(texture, None, Some(&mut view))?;
    view.context("创建 Shader Resource View 未返回对象")
}

unsafe fn create_render_target(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
) -> Result<ID3D11RenderTargetView> {
    let mut target = None;
    device.CreateRenderTargetView(texture, None, Some(&mut target))?;
    target.context("创建 Render Target View 未返回对象")
}

unsafe fn compile_shader(entry: &[u8], target: &[u8]) -> Result<ID3DBlob> {
    let entry = CStr::from_bytes_with_nul(entry).context("Shader 入口点不是 C 字符串")?;
    let target = CStr::from_bytes_with_nul(target).context("Shader Profile 不是 C 字符串")?;
    let mut code = None;
    let mut errors = None;
    let result = D3DCompile(
        SHADER.as_ptr().cast::<c_void>(),
        SHADER.len(),
        PCSTR::null(),
        None,
        None,
        PCSTR(entry.as_ptr().cast()),
        PCSTR(target.as_ptr().cast()),
        1 << 15,
        0,
        &mut code,
        Some(&mut errors),
    );
    if let Err(error) = result {
        let details = errors
            .as_ref()
            .map(|blob| String::from_utf8_lossy(blob_bytes(blob)).into_owned())
            .unwrap_or_default();
        return Err(anyhow!("编译 Shader 失败: {error}; {details}"));
    }
    code.context("Shader 编译成功但未返回字节码")
}

unsafe fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize())
}

fn point_in_rect(x: i32, y: i32, rect: &WinRect) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    #[test]
    fn point_in_rect_supports_negative_monitor_coordinates() {
        let rect = WinRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        assert!(point_in_rect(-960, 540, &rect));
        assert!(!point_in_rect(0, 540, &rect));
    }

    #[test]
    fn constants_are_sixteen_byte_aligned_in_size() {
        assert_eq!(size_of::<GlassConstants>() % 16, 0);
    }

    #[test]
    #[ignore = "依赖当前 Windows 图形会话和可用的 Desktop Duplication"]
    fn desktop_capture_and_shader_smoke_test() {
        let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        let width = 320;
        let height = 240;
        let mut renderer = match GlassRenderer::new() {
            Ok(renderer) => renderer,
            Err(error) => panic!("液态玻璃渲染器初始化失败: {error:#}"),
        };
        let pixels = match renderer.render(
            (screen_width - width) / 2,
            (screen_height - height) / 2,
            width as u32,
            height as u32,
            16.0,
        ) {
            Ok(pixels) => pixels,
            Err(error) => panic!("液态玻璃测试帧生成失败: {error:#}"),
        };

        assert_eq!(pixels.len(), width as usize * height as usize);
        assert!(pixels.iter().any(|pixel| pixel.a > 0));
        assert!(pixels.iter().any(|pixel| pixel.a == 0));

        if let Ok(path) = std::env::var("IKUN_GLASS_SMOKE_OUTPUT") {
            write_preview_bmp(&path, &pixels, width as u32, height as u32);
        }
    }

    #[test]
    #[ignore = "依赖当前 Windows D3D11 图形会话"]
    fn quadrant_background_moves_and_blends_continuously() {
        let mut renderer = match GlassRenderer::new() {
            Ok(renderer) => renderer,
            Err(error) => panic!("液态玻璃渲染器初始化失败: {error:#}"),
        };
        let source_width = 800_u32;
        let source_height = 600_u32;
        let source_pixels = quadrant_pixels(source_width, source_height);
        let source_view = unsafe {
            create_test_source_view(
                &renderer.device,
                &source_pixels,
                source_width,
                source_height,
            )
        };
        let source_view = match source_view {
            Ok(view) => view,
            Err(error) => panic!("创建四分区测试纹理失败: {error:#}"),
        };

        let width = 240_u32;
        let height = 180_u32;
        let positions = [(40, 40), (520, 40), (40, 380), (520, 380)];
        let mut samples = Vec::new();
        for (x, y) in positions {
            let pixels = render_test_region(
                &mut renderer,
                &source_view,
                x,
                y,
                width,
                height,
                source_width,
                source_height,
            );
            samples.push(pixels[(height / 2 * width + width / 2) as usize]);
        }
        assert!(samples[0].r > samples[0].g && samples[0].r > samples[0].b);
        assert!(samples[1].r > 180 && samples[1].g > 180 && samples[1].b < 80);
        assert!(samples[2].b > samples[2].r && samples[2].b > samples[2].g);
        assert!(samples[3].g > samples[3].r && samples[3].g > samples[3].b);

        let movement_positions = [250, 280, 310];
        let mut green_averages = Vec::new();
        let mut preview_frames = Vec::new();
        for x in movement_positions {
            let pixels = render_test_region(
                &mut renderer,
                &source_view,
                x,
                40,
                width,
                height,
                source_width,
                source_height,
            );
            green_averages.push(channel_average(&pixels, |pixel| pixel.g));
            preview_frames.push(pixels);
        }
        assert!(green_averages[0] < green_averages[1]);
        assert!(green_averages[1] < green_averages[2]);

        let boundary = &preview_frames[1];
        let center = boundary[(height / 2 * width + width / 2) as usize];
        assert!(center.r > 100 && center.g > 20);

        if let Ok(path) = std::env::var("IKUN_GLASS_QUADRANT_OUTPUT") {
            let combined = combine_horizontal(&preview_frames, width, height);
            write_preview_bmp(&path, &combined, width * 3, height);
        }
    }

    fn render_test_region(
        renderer: &mut GlassRenderer,
        source_view: &ID3D11ShaderResourceView,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        source_width: u32,
        source_height: u32,
    ) -> Vec<Pixel32> {
        match renderer.render_view(
            source_view,
            [
                x as f32 / source_width as f32,
                y as f32 / source_height as f32,
                width as f32 / source_width as f32,
                height as f32 / source_height as f32,
            ],
            [source_width as f32, source_height as f32],
            width,
            height,
            16.0,
        ) {
            Ok(pixels) => pixels,
            Err(error) => panic!("四分区液态玻璃渲染失败: {error:#}"),
        }
    }

    unsafe fn create_test_source_view(
        device: &ID3D11Device,
        pixels: &[Pixel32],
        width: u32,
        height: u32,
    ) -> Result<ID3D11ShaderResourceView> {
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
            pSysMem: pixels.as_ptr().cast(),
            SysMemPitch: width * size_of::<Pixel32>() as u32,
            ..Default::default()
        };
        let mut texture = None;
        device.CreateTexture2D(&desc, Some(&initial), Some(&mut texture))?;
        let texture = texture.context("创建四分区纹理未返回对象")?;
        create_shader_view(device, &texture)
    }

    fn quadrant_pixels(width: u32, height: u32) -> Vec<Pixel32> {
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = match (x < width / 2, y < height / 2) {
                    (true, true) => (255, 0, 0),
                    (false, true) => (255, 255, 0),
                    (true, false) => (0, 0, 255),
                    (false, false) => (0, 255, 0),
                };
                pixels.push(Pixel32 { b, g, r, a: 255 });
            }
        }
        pixels
    }

    fn channel_average(pixels: &[Pixel32], channel: impl Fn(&Pixel32) -> u8) -> f32 {
        pixels
            .iter()
            .map(|pixel| channel(pixel) as u64)
            .sum::<u64>() as f32
            / pixels.len() as f32
    }

    fn combine_horizontal(frames: &[Vec<Pixel32>], width: u32, height: u32) -> Vec<Pixel32> {
        let mut combined = Vec::with_capacity(width as usize * height as usize * frames.len());
        for row in 0..height as usize {
            for frame in frames {
                combined
                    .extend_from_slice(&frame[row * width as usize..(row + 1) * width as usize]);
            }
        }
        combined
    }

    fn write_preview_bmp(path: &str, pixels: &[Pixel32], width: u32, height: u32) {
        let row_stride = (width * 3).div_ceil(4) * 4;
        let pixel_bytes = row_stride * height;
        let file_size = 54 + pixel_bytes;
        let mut file = std::fs::File::create(path)
            .unwrap_or_else(|error| panic!("创建液态玻璃预览失败: {error}"));
        let mut header = [0_u8; 54];
        header[0..2].copy_from_slice(b"BM");
        header[2..6].copy_from_slice(&file_size.to_le_bytes());
        header[10..14].copy_from_slice(&54_u32.to_le_bytes());
        header[14..18].copy_from_slice(&40_u32.to_le_bytes());
        header[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        header[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        header[26..28].copy_from_slice(&1_u16.to_le_bytes());
        header[28..30].copy_from_slice(&24_u16.to_le_bytes());
        header[34..38].copy_from_slice(&pixel_bytes.to_le_bytes());
        file.write_all(&header)
            .unwrap_or_else(|error| panic!("写入液态玻璃预览头失败: {error}"));

        let padding = vec![0_u8; (row_stride - width * 3) as usize];
        for row in (0..height as usize).rev() {
            for pixel in &pixels[row * width as usize..(row + 1) * width as usize] {
                let inverse_alpha = 255_u16 - pixel.a as u16;
                let checker = 48_u16;
                let b = pixel.b as u16 + checker * inverse_alpha / 255;
                let g = pixel.g as u16 + checker * inverse_alpha / 255;
                let r = pixel.r as u16 + checker * inverse_alpha / 255;
                file.write_all(&[b.min(255) as u8, g.min(255) as u8, r.min(255) as u8])
                    .unwrap_or_else(|error| panic!("写入液态玻璃预览像素失败: {error}"));
            }
            file.write_all(&padding)
                .unwrap_or_else(|error| panic!("写入液态玻璃预览行失败: {error}"));
        }
    }
}
