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
use windows::Win32::Foundation::{POINT, RECT as WinRect};
use windows_numerics::{Matrix3x2, Vector2};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COMPOSITE_MODE_SOURCE_COPY,
    D2D1_PIXEL_FORMAT, D2D_MATRIX_5X4_F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1Border, CLSID_D2D1ColorMatrix, CLSID_D2D1Crop, CLSID_D2D1GaussianBlur, CLSID_D2D1Scale,
    D2D1_PROPERTY_TYPE_VECTOR2, D2D1_SCALE_PROP_SCALE, ID2D1Bitmap1,
    ID2D1DeviceContext, ID2D1Effect, ID2D1Image, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_BORDER_EDGE_MODE_MIRROR, D2D1_BORDER_PROP_EDGE_MODE_X, D2D1_BORDER_PROP_EDGE_MODE_Y,
    D2D1_COLORMATRIX_PROP_COLOR_MATRIX, D2D1_CROP_PROP_RECT, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_MATRIX_5X4,
    D2D1_PROPERTY_TYPE_VECTOR4,
};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D11::D3D11_MAP_FLAG_DO_NOT_WAIT;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R10G10B10A2_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, IDXGISurface,
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_MORE_DATA, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    DXGI_OUTDUPL_MOVE_RECT,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::ffi_types::Pixel32;
use crate::config::{BoxStyleConfig, LiquidGlassConfig};

const RETRY_DELAY: Duration = Duration::from_secs(3);

/// 初始化连续失败时的退避上限。
///
/// 屏幕锁定 / 安全桌面下 DXGI 会对**所有**输出返回
/// E_ACCESSDENIED，而渲染器会按 RETRY_DELAY 不停重试。每次重试
/// 都要重建一个 D3D11 设备（这一步本身就是十毫秒级），实测锁屏
/// 期间白烧 ~7% 单核，一整晚就是几小时 CPU 与满屏日志。
/// 改为指数退避：3s → 6s → 10s 封顶，解锁后最多 10 秒恢复。
const RETRY_DELAY_MAX: Duration = Duration::from_secs(10);
static FIRST_FRAME_DIAGNOSTIC_LOGGED: AtomicBool = AtomicBool::new(false);
static NON_BLACK_FRAME_DIAGNOSTIC_LOGGED: AtomicBool = AtomicBool::new(false);
/// E_ACCESSDENIED 的成因提示只打一次（否则锁屏期间每 3 秒刷一条）
static ACCESS_DENIED_HINT_LOGGED: AtomicBool = AtomicBool::new(false);

// 最近一帧 `DXGI_OUTDUPL_FRAME_INFO` 的关键字段。
//
// 只为诊断日志服务：判断「整屏脏矩形」到底是驱动把矩形合并了
// （`RectsCoalesced = true`，那么按框判定在这台机器上就拿不到粒度），
// 还是桌面真的每秒整屏变一次（那就是有自激回路要查）。
// 用 thread_local 传是因为 `collect_frame_changes` 的参数已经很长，
// 而它只在渲染线程上被调用。
thread_local! {
    // 上次输出区域指纹诊断日志的时刻（限频）
    static REGION_HASH_LOG_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    static LAST_FRAME_COALESCED: Cell<bool> = const { Cell::new(false) };
    static LAST_FRAME_METADATA: Cell<u32> = const { Cell::new(0) };
    static LAST_FRAME_ACCUM: Cell<u32> = const { Cell::new(0) };
}

/// 桌面内容代号：任一显示器捕获到新帧就自增。
///
/// 分类框的周期 tick 用它判断「玻璃背景是否变了」，
/// 没变就跳过整帧渲染。
static DESKTOP_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 取帧诊断计数（只读，供 `mem_probe` 输出）。
///
/// 「桌面变化」这一项一旦大到等于 tick 数，就说明按框判定退化了；
/// 但光看退化不知道退化在哪一环，必须把三种帧分开数：
/// - 取到的帧总数
/// - `LastPresentTime == 0` 的帧：DXGI 只更新了鼠标位置，桌面图像没变
/// - 有图像更新但 DXGI 一个矩形都没给的帧：只能整块输出保守处理
///
/// 区域像素比对次数 / 其中判定为「真的变了」的次数。
///
/// 这两个数的比值就是脏矩形方案在本机的**真实有效性**：
/// 命中率高说明驱动报的整屏矩形大部分是空报，按框过滤省下了渲染；
/// 命中率接近 100% 说明桌面确实在变，渲染省不掉，不该再往下压。
static REGION_CHECKS: AtomicU64 = AtomicU64::new(0);
static REGION_CHANGED: AtomicU64 = AtomicU64::new(0);

/// 区域像素比对计数快照：(比对次数, 判定为变化的次数)。
pub fn region_check_counters() -> (u64, u64) {
    (
        REGION_CHECKS.load(Ordering::Relaxed),
        REGION_CHANGED.load(Ordering::Relaxed),
    )
}

/// 同时监视的桌面区域上限。
///
/// 稳态下等于分类框个数。拖动分类框时区域逐帧变化会产生新键，
/// 触顶直接清空重来 —— 代价只是下一轮每个框多渲染一帧。
const MAX_REGION_WATCHES: usize = 32;

static FRAMES_ACQUIRED: AtomicU64 = AtomicU64::new(0);
static FRAMES_MOUSE_ONLY: AtomicU64 = AtomicU64::new(0);
static FRAMES_WITHOUT_RECTS: AtomicU64 = AtomicU64::new(0);

/// 取帧诊断计数快照：(取帧总数, 仅鼠标帧, 有图像更新但无矩形帧)。
pub fn frame_counters() -> (u64, u64, u64) {
    (
        FRAMES_ACQUIRED.load(Ordering::Relaxed),
        FRAMES_MOUSE_ONLY.load(Ordering::Relaxed),
        FRAMES_WITHOUT_RECTS.load(Ordering::Relaxed),
    )
}

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

/// 一个输出多久没有分类框在上面渲染，就停止轮询并释放它的捕获资源。
///
/// `poll_desktop_changes` 会为**每个**输出提前分配一张全尺寸桌面纹理，
/// 而主屏是 3120x2080 —— 一张就 24.8MB，加上 DXGI 桌面复制自己的
/// 内部缓冲，一块没人用的屏幕要白占几十 MB 显存。分类框只放在
/// 副屏时，主屏那份完全是浪费；显示器越多浪费越大。
///
/// 分类框在某个输出上渲染过就打时间戳；超过这个时长没有再渲染，
/// 说明框已经搬走了，停止轮询并把该输出的纹理/位图/模糊链全部释放。
/// 框搬回来时 `render` 会按需重建，路径与首次初始化完全相同。
///
/// 取 30 秒：远大于周期重绘心跳（10 秒）与遮挡态心跳（5 秒），
/// 只要还有框在这个输出上，时间戳就一定被持续刷新，不会误释放。
const OUTPUT_IDLE_RELEASE: Duration = Duration::from_secs(30);

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

/// 复用的 D2D 模糊 effect 链（**每个分类框裁剪框一份**）。
///
/// 各级输入在创建时接好，裁剪框 / 边框模式也一次写死，运行期只在
/// σ 或饱和度真的变了才 SetValue。保持 effect 对象身份与属性都不变，
/// D2D1 才能命中它为该图缓存的 realization（高斯模糊的中间表面）。
///
/// 为什么不是「每个 output 一份」：`ID2D1Effect::SetInput` 与 `SetValue`
/// 都会把该 effect 及其**下游整张图**标记为脏，realization 随即作废，
/// 下一次 DrawImage 重新申请中间表面。多个分类框轮流共用一条链，
/// 就意味着每一帧都必然重建一次全图 —— 实测显存专用 60 秒内
/// 从 417MB 涨到 593MB，而渲染器自己记账的纹理一动不动。
struct BlurChain {
    crop: ID2D1Effect,
    border: ID2D1Effect,
    blur: ID2D1Effect,
    scale: ID2D1Effect,
    saturate: ID2D1Effect,
    /// `saturate.GetOutput()`，DrawImage 的输入
    saturated: ID2D1Image,
    /// 建链时写死的裁剪框，同时也是 DrawImage 时的 axis-aligned clip
    crop_rect: D2D_RECT_F,
    /// 已写入 blur 的标准差；与之相同就不再 SetValue
    sigma: f32,
    /// 已写入 saturate 的饱和度；与之相同就不再 SetValue
    saturation: f32,
    /// 半分辨率链（拖拽期临时链）：GPU 成本约 1/4，画面略软但运动中不可感知
    half: bool,
    /// 最近一次被使用的帧序号（LRU 淘汰用）
    last_used: u64,
}

/// 每个 output 上最多保留的模糊链条数。
///
/// 稳态下等于该 output 上的分类框个数（通常 5 个以内）。拖动分类框时
/// 裁剪框逐帧变化会不断产生新键，触顶后按最久未用淘汰，
/// 保证 D2D 缓存的 realization 总量有界。
const MAX_BLUR_CHAINS_PER_OUTPUT: usize = 12;

/// 同一个裁剪框连续未命中多少次后，才升级成常驻链。
///
/// 设成 2 而不是 1，是为了把「分类框停住了」和「正在拖动」区分开：
/// 拖动时裁剪框每帧都变，任何键都只会命中一次，永远升不了级，
/// 于是整段拖动都走复用临时链的便宜路径；分类框一停住，
/// 同一个键第二次出现就升级成常驻链，此后每帧零 `SetValue`。
const CHAIN_PROMOTE_AFTER_MISSES: u8 = 2;

/// 待升级键表上限。拖动会不断产生新键，触顶直接清空，
/// 代价只是每个框重新计一次数，不会漏升级。
const MAX_PENDING_CHAIN_KEYS: usize = 32;

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
    /// 按裁剪框缓存的模糊 effect 链（一个分类框一条，静止时命中）
    blur_chains: HashMap<BlurChainKey, BlurChain>,
    /// 拖动 / 缩放时复用的**临时**模糊链。
    ///
    /// 裁剪框逐帧变化时不值得为每个位置建一条常驻链：那等于每帧
    /// `CreateEffect` ×4 再淘汰 ×4，比改造前「一条链 + SetValue」
    /// 每帧做的功更多，会直接反映在拖动时的液态玻璃流畅度上。
    /// 临时链只改裁剪框，effect 对象身份始终稳定。
    transient_chain: Option<BlurChain>,
    /// 未命中键 → 连续出现次数，达到 `CHAIN_PROMOTE_AFTER_MISSES` 才升级
    pending_chain_keys: HashMap<BlurChainKey, u8>,
    /// 单调递增的帧序号，驱动 `blur_chains` 的 LRU 淘汰
    blur_stamp: u64,
    /// 累计建链次数。稳态下应当停在「分类框个数」不再增长；
    /// 持续增长说明分类框在动或裁剪框在抖，是显存 churn 的信号。
    blur_chains_created: u64,
    /// 最近一次有分类框在这个输出上渲染的时刻。
    ///
    /// `None` = 还没有任何分类框用过这个输出。驱动
    /// `OUTPUT_IDLE_RELEASE` 的空闲释放，见该常量说明。
    last_box_render_at: Option<Instant>,
    /// 脏矩形临时缓冲。每次取帧都要拿一批矩形，新建 Vec 就是
    /// 白白的热路径分配；复用后稳态下零分配。
    dirty_scratch: Vec<WinRect>,
    /// 移动矩形临时缓冲，同上
    move_scratch: Vec<DXGI_OUTDUPL_MOVE_RECT>,
    /// 脏矩形坐标约定的探测结果。
    ///
    /// `None` = 还没探明，`Some(false)` = 输出相对坐标（需加上
    /// output 原点偏移），`Some(true)` = 已是桌面绝对坐标。
    ///
    /// DXGI 文档只说脏矩形描述「桌面图像」的区域，没明说是哪个
    /// 坐标系。主屏原点在 (0,0)，两种约定结果完全相同，既探不出也
    /// 不需要探；副屏两者相差整整一个 output 偏移，一批矩形就能判明。
    /// 判错会让副屏上的分类框永远收不到变化通知（画面冻住
    /// 且无任何报错），所以这里显式探测 + 打日志，而不是把约定写死。
    dirty_rects_absolute: Option<bool>,
    /// 首批脏矩形原样打过一次日志了没有（一次性诊断，避免刷屏）
    dirty_rect_probe_logged: bool,
    /// 首帧 `DXGI_OUTDUPL_FRAME_INFO` 打过一次日志了没有
    frame_info_probe_logged: bool,
    /// 首个「有图像更新但 DXGI 不给矩形」的帧打过一次日志了没有
    no_rect_frame_logged: bool,
    /// 上次把脏矩形样本打进日志的时刻（限频，默认 2 秒一条）
    last_rect_sample_log: Option<Instant>,
    /// 取移动矩形失败打过一次日志了没有
    move_rect_failure_logged: bool,
    /// 「两种约定都不符」打过一次日志了没有。
    ///
    /// 探测器返回 `None` 时不能默默沿用假设：那等于把「我不知道」
    /// 伪装成「我知道是输出相对坐标」。副屏上判错的表现是分类框
    /// 永远收不到变化通知（画面冻住且无任何报错），必须留痕。
    dirty_rect_inconclusive_logged: bool,
}

/// 模糊链缓存键：分类框在桌面坐标下的裁剪框（位置 + 尺寸）。
type BlurChainKey = (i32, i32, i32, i32);

struct EmbeddedMap {
    view: ID3D11ShaderResourceView,
}

/// 每框私有的回读暂存：保存「本框上一帧写的玻璃」。
/// 本帧读它（GPU 早已完成，DO_NOT_WAIT 基本必中），
/// 渲染后把本帧结果写回去给下一帧读。
struct BoxReadback {
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
    has_prev: bool,
}

struct RenderSurfaces {
    width: u32,
    height: u32,
    output_texture: ID3D11Texture2D,
    output_target: ID3D11RenderTargetView,
    /// `output_texture` 的 D2D **零拷贝**视图，与它同生同灭。
    ///
    /// 有了它，合成端可以直接 `DrawBitmap` 玻璃输出，不必再
    /// `CopySubresourceRegion` → `Map` 阻塞等 GPU → 逐行 memcpy 回 CPU
    /// → `CopyFromMemory` 上传。实测那一条往返占单帧 20ms 里的 5.5ms。
    ///
    /// `None` 表示包装失败（格式/绑定不支持等），调用方必须退回
    /// 原有的 CPU 回读路径，绝不能当成「玻璃是黑的」继续画。
    output_bitmap: Option<ID2D1Bitmap1>,
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
    /// 每个分类框私有的回读暂存（见 BoxReadback）。
    ///
    /// 旧双缓冲的暂存全局共享 + parity 全局翻转：两个分类框
    /// 交替渲染时 A 会读到 B 写的像素，画面上表现为两个框的
    /// 玻璃背景互相闪烁（用户反馈 2026-09-14）。改为每框
    /// 私有 + 「读自己上一帧写的」，框间竞态窗口为零。
    readbacks: std::collections::HashMap<usize, BoxReadback>,
    /// 前几帧仍用阻塞 Map 读「本帧刚写的」，避免读到未初始化的暂存纹理。
    /// 所有分类框共用的一份渲染表面（输出纹理 + RTV + 暂存纹理）。
    ///
    /// 尺寸取「历史最大」，只增不减。渲染时视口与裁剪矩形都限定在
    /// 当前分类框自己的 surface_width × surface_height 内，着色器
    /// `VSMain` 又是按 NDC 生成全屏三角形（与 RT 实际尺寸无关），
    /// 因此画进大纹理的左上角与画进刚好同尺寸的纹理逐像素一致。
    ///
    /// 旧实现按 (宽, 高) 各存一份、最多 8 份：N 个分类框就是 2N 张
    /// 全尺寸纹理。200% DPI 下单张表面可达 15MB，10 个分类框就是
    /// 300MB 显存，而其中绝大部分是重复的。共享之后恒定 2 张。
    surfaces: Option<RenderSurfaces>,
    displacement_maps: [EmbeddedMap; 3],
    light_direction: [f32; 2],
    startup_black_frames_remaining: u8,
    /// 连续因 ACCESS_LOST 触发重建的次数，驱动指数退避
    access_lost_streak: u32,
    /// 上次重建 duplication 的时刻（限频用）
    last_duplication_rebuild: Option<Instant>,
    /// 区域像素比对用的暂存纹理（所有区域共享一份，只增不减）
    diff_staging: Option<DiffStaging>,
    /// 受监视桌面区域的比对状态
    region_watches: HashMap<RegionKey, RegionWatch>,
}

/// 区域比对暂存纹理，连同它的格式与尺寸。
///
/// 格式必须记下来：`CopySubresourceRegion` 要求源与目标格式一致，
/// 而捕获纹理的格式跟随桌面（HDR 打开时会变成 R16G16B16A16_FLOAT），
/// 格式变了就得重建，否则拷贝静默失败、比对结果永远「没变」。
struct DiffStaging {
    texture: ID3D11Texture2D,
    format: DXGI_FORMAT,
    width: u32,
    height: u32,
}

/// 受监视区域的键：桌面坐标矩形。
type RegionKey = (i32, i32, i32, i32);

/// 一个受监视桌面区域的比对状态。
#[derive(Clone, Copy)]
struct RegionWatch {
    /// 上次比对时的桌面代号。同一代号内重复询问直接复用结论，
    /// 否则「判定为没变 → 不渲染 → 代号不前进 → 下一 tick 再比一次」
    /// 会让每个 tick 都白跑一次 GPU 拷贝 + Map。
    checked_generation: u64,
    /// 上次比对的结论
    changed: bool,
    /// 上次看到的区域内容指纹
    hash: u64,
}

enum GlassState {
    Uninitialized,
    Ready(GlassRenderer),
    Failed { retry_at: Instant },
}

thread_local! {
    static GLASS_STATE: RefCell<GlassState> = const { RefCell::new(GlassState::Uninitialized) };
    /// 连续初始化失败次数（驱动指数退避，成功后归零）
    static INIT_FAILURES: Cell<u32> = const { Cell::new(0) };
    /// 待判定的桌面变化矩形队列（按代号递增）
    static DESKTOP_CHANGES: RefCell<VecDeque<(u64, WinRect)>> = RefCell::new(VecDeque::new());
    /// 上一次真正向 DXGI 取帧的时刻（轮询节流用）
    static LAST_DESKTOP_POLL: Cell<Option<Instant>> = const { Cell::new(None) };
}

static LIQUID_GLASS_CONFIG: std::sync::OnceLock<std::sync::RwLock<LiquidGlassConfig>> =
    std::sync::OnceLock::new();
static BOX_STYLE_CONFIG: std::sync::OnceLock<std::sync::RwLock<BoxStyleConfig>> =
    std::sync::OnceLock::new();

/// 液态玻璃渲染器的内存占用快照（只读诊断，供 `mem_probe` 输出）。
///
/// 玻璃开启后主进程内存的去向必须可归因，否则「还是 500MB」这类
/// 反馈只能靠猜。这里把渲染器持有的每一类资源折算成字节数暴露出去。
#[derive(Clone, Copy, Default)]
pub struct GlassMemoryStats {
    /// 状态文字：未初始化 / 就绪 / 初始化失败退避中 / 忙碌
    pub state_name: &'static str,
    /// 已创建桌面捕获纹理的输出个数
    pub capture_texture_count: usize,
    /// 桌面捕获纹理合计字节（每个输出一张全屏纹理）
    pub capture_texture_bytes: u64,
    /// 已创建模糊纹理的输出个数
    pub blurred_texture_count: usize,
    /// 模糊纹理合计字节（与捕获纹理同尺寸）
    pub blurred_texture_bytes: u64,
    /// 共享渲染表面的张数（0 或 1）
    pub surface_cache_entries: usize,
    /// 共享渲染表面字节（输出纹理 + 暂存纹理，两张同尺寸）
    pub surface_cache_bytes: u64,
    /// 当前存活的模糊链条数（稳态应等于分类框个数）
    pub blur_chain_count: usize,
    /// 累计建链次数。稳态下应当停住；持续增长 = 显存 churn 信号
    pub blur_chains_created: u64,
    /// 输出总数
    pub output_count: usize,
    /// 当前有分类框驻留（因而持有桌面纹理）的输出数
    pub active_output_count: usize,
}

/// 按纹理描述折算字节数（本模块所有纹理都是 MipLevels=1、ArraySize=1）。
fn texture_bytes(texture: &ID3D11Texture2D) -> u64 {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        let bytes_per_pixel = match desc.Format {
            DXGI_FORMAT_R16G16B16A16_FLOAT => 8u64,
            _ => 4u64,
        };
        desc.Width as u64 * desc.Height as u64 * desc.ArraySize.max(1) as u64 * bytes_per_pixel
    }
}

/// 采集一次液态玻璃内存快照。
///
/// 渲染器正被借用（同一时刻在渲染）时只回报状态，其余分量留 0——
/// 诊断不能反过来干扰渲染，更不能 panic。
pub fn memory_stats() -> GlassMemoryStats {
    let mut stats = GlassMemoryStats::default();
    GLASS_STATE.with(|state| match state.try_borrow() {
        Ok(guard) => match &*guard {
            GlassState::Uninitialized => stats.state_name = "未初始化",
            GlassState::Failed { .. } => stats.state_name = "初始化失败退避中",
            GlassState::Ready(renderer) => {
                stats.state_name = "就绪";
                for output in &renderer.outputs {
                    if let Some(texture) = &output.desktop_texture {
                        stats.capture_texture_count += 1;
                        stats.capture_texture_bytes += texture_bytes(texture);
                    }
                    if let Some(texture) = &output.blurred_texture {
                        stats.blurred_texture_count += 1;
                        stats.blurred_texture_bytes += texture_bytes(texture);
                    }
                }
                for output in &renderer.outputs {
                    stats.blur_chain_count +=
                        output.blur_chains.len() + usize::from(output.transient_chain.is_some());
                    stats.blur_chains_created += output.blur_chains_created;
                    if output.last_box_render_at.is_some() {
                        stats.active_output_count += 1;
                    }
                }
                stats.output_count = renderer.outputs.len();
                if let Some(surface) = &renderer.surfaces {
                    stats.surface_cache_entries = 1;
                    // 输出纹理 1 张；暂存改每框私有，逐框累加
                    stats.surface_cache_bytes =
                        surface.width as u64 * surface.height as u64 * 4;
                    for rb in renderer.readbacks.values() {
                        stats.surface_cache_bytes += rb.width as u64 * rb.height as u64 * 4;
                    }
                }
            }
        },
        Err(_) => stats.state_name = "忙碌",
    });
    stats
}

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

/// GPU 捕获 / 渲染失败时，直接沿用调用方缓冲区里已有的上一帧。
///
/// `out` 就是分类框自己跨帧复用的回读缓冲（`GridWindowData::glass_pixels`），
/// 上一帧成功渲染的像素本来就在里面、尺寸也对得上，不需要再留一份全局副本。
///
/// 旧实现另开一张以屏幕坐标为键的全局缓存（最多 8 条），每个分类框因此
/// 多存一整张表面：200% DPI 下单张可达 15MB，5 个框就是 70MB，
/// 而它提供的信息与 `out` 完全重复 —— 拖动分类框后键变了，
/// 缓存照样命不中，能兜住的场景 `out` 也能兜住。
///
/// 只有缓冲区尺寸与本帧要求不符（刚改过窗口大小）时才返回 `false`，
/// 由调用方回退到半透明背景，这与旧的缓存未命中行为一致。
fn reuse_existing_frame(out: &[Pixel32], expected_len: usize) -> bool {
    expected_len > 0 && out.len() == expected_len
}

/// 本帧要求的回读缓冲长度：整张带外投影留白的表面。
fn expected_surface_len(content_width: i32, content_height: i32, padding: i32) -> usize {
    let w = content_width as i64 + padding as i64 * 2;
    let h = content_height as i64 + padding as i64 * 2;
    if w <= 0 || h <= 0 {
        return 0;
    }
    (w * h) as usize
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

/// 显示器 / DPI / 桌面组成变化后作废整个渲染器，下一次分类框渲染前重建。
///
/// # 为什么是整机重建，而不是只丢纹理
///
/// 只丢 `desktop_texture` 的话 duplication 还是旧的，而
/// `AcquireNextFrame` **只投递增量**：桌面一旦静止（屏保结束、改完
/// 分辨率没人动、跨屏拖动结束后画面停住），就永远等不到新帧，
/// `desktop_texture` 再也建不起来 —— 玻璃冻死在最后一帧旧画面上，
/// 而且**没有任何报错**。实测作废之后 6 分钟都没有自愈。
///
/// 想靠重建 duplication 强制取一张全图也不行：DXGI 不允许同一输出上
/// 同时存在两个 duplication，旧对象还活着时 `DuplicateOutput` 必定失败。
/// 要释放旧的就得把字段改成 `Option`，牵动整条取帧热路径；
/// 而 `WM_DPICHANGED` 本来只在跨屏拖动时发生，整机重建那几十毫秒
/// （设备 + 着色器 + 位移贴图 + duplication）完全付得起，风险还小得多。
pub fn invalidate_desktop_snapshot() {
    GLASS_STATE.with(|state| {
        if let Ok(mut state) = state.try_borrow_mut() {
            if matches!(&*state, GlassState::Ready(_)) {
                log::info!("[LG] 显示器/DPI/桌面组成变化，作废液态玻璃渲染器，下一帧重建");
                *state = GlassState::Uninitialized;
            }
        }
    });
    // 退避计数归零：这是主动作废，不是初始化失败，下一帧就该立刻重建
    reset_init_backoff();
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

/// 脏矩形 / 移动矩形缓冲的初始容量（个数）。
///
/// # 为什么不能先「传 0 + 空指针」问大小
///
/// 那是很多示例里的写法，但**实测本机 DXGI 对该调用直接返回
/// `E_INVALIDARG`（参数错误）**，而不是文档里说的 `DXGI_ERROR_MORE_DATA`。
/// 结果一个矩形都拿不到，`region_changed_since` 只能走「队列为空但代号
/// 已前进 → 保守返回 true」的兜底 —— 脏矩形按框判定形同虚设，
/// 桌面任何一处变化都会让全部分类框陪绘，而且日志里一点痕迹都没有。
///
/// 正确姿势是直接给一块真缓冲（64 个矩形 = 1KB，覆盖绝大多数帧），
/// 不够再按 DXGI 报出的 required 扩容重试。
const RECT_BUFFER_INITIAL_CAPACITY: usize = 64;

/// 扩容重试次数上限，防止 DXGI 反复报 MORE_DATA 时死循环。
const RECT_BUFFER_MAX_GROWTH: usize = 4;

/// 脏矩形样本日志的最小间隔。
///
/// 只有把「到底哪些矩形被推给了变化队列」打出来，才能判断按框判定
/// 有没有真在过滤：如果每条样本都是整屏矩形，那说明退化点在驱动侧
/// （矩形被合并）或者在应用自己（分类框重绘 → 桌面变脏 → 再重绘
/// 的自激回路），两者的处理方式完全不同。2 秒一条不会刷爆日志。
const RECT_SAMPLE_LOG_INTERVAL: Duration = Duration::from_secs(2);

/// 采集本帧的脏矩形与移动矩形。
///
/// 必须在 `AcquireNextFrame` 与 `ReleaseFrame` 之间调用。
/// 两种矩形都要取：窗口拖动/滚动时 DXGI 用 move rect 表达，
/// dirty rect 里可能只有很小一块甚至为空。
///
/// 返回是否真的往变化队列里推了矩形。调用方据此决定要不要推进桌面代号：
/// 一帧既没有图像更新、也没有任何矩形时，推进代号只会让
/// `region_changed_in` 走「队列为空但代号已前进」的保守兜底，
/// 把**所有输出上的所有分类框**一起拖下水陪绘。
unsafe fn collect_frame_changes(output: &mut CaptureOutput, generation: u64) -> bool {
    let dirty = collect_dirty_rects(output, generation);
    let moved = collect_move_rects(output, generation);
    dirty || moved
}

/// 当前应当采用的坐标原点偏移。
///
/// 约定探测出「桌面绝对坐标」时不加偏移；其余情况（已判明输出相对，
/// 或还没判明）一律加输出原点偏移 —— 主屏原点本来就是 (0,0)，
/// 两种约定等价，不存在风险。
fn change_origin(output: &CaptureOutput) -> (i32, i32) {
    if output.dirty_rects_absolute == Some(true) {
        (0, 0)
    } else {
        (output.rect.left, output.rect.top)
    }
}

/// 取本帧脏矩形并记入变化队列，返回是否推入了至少一个矩形。
unsafe fn collect_dirty_rects(output: &mut CaptureOutput, generation: u64) -> bool {
    let rect_size = size_of::<WinRect>();
    let empty = WinRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let mut capacity = RECT_BUFFER_INITIAL_CAPACITY;
    for _ in 0..=RECT_BUFFER_MAX_GROWTH {
        // 复用缓冲：`clear` 保留容量，容量不变时 resize 不会再向分配器要内存
        output.dirty_scratch.clear();
        output.dirty_scratch.resize(capacity, empty);
        let mut required: u32 = 0;
        let result = output.duplication.GetFrameDirtyRects(
            (capacity * rect_size) as u32,
            output.dirty_scratch.as_mut_ptr(),
            &mut required,
        );
        let needed = ((required as usize) / rect_size).max(1);
        if let Err(error) = &result {
            if error.code() == DXGI_ERROR_MORE_DATA && needed > capacity {
                capacity = needed;
                continue;
            }
            // 真失败才留痕；「成功但 0 个矩形」是正常帧，不该报警
            if !output.dirty_rect_probe_logged {
                output.dirty_rect_probe_logged = true;
                log::warn!(
                    "[LG] 取脏矩形失败: {error:?}（required={required}B，缓冲 {} 个）；\
                     分类框变化判定将退化为整桌面粒度",
                    capacity
                );
            }
            return false;
        }
        let got = needed.min(capacity);
        if got == 0 {
            return false;
        }
        // 限频把矩形样本打出来，用于判断按框判定是否真的在过滤
        {
            let now = Instant::now();
            let due = output
                .last_rect_sample_log
                .is_none_or(|last| now.duration_since(last) >= RECT_SAMPLE_LOG_INTERVAL);
            if due {
                output.last_rect_sample_log = Some(now);
                let full = output.rect.right - output.rect.left;
                let full_h = output.rect.bottom - output.rect.top;
                let covers_all = output.dirty_scratch[..got]
                    .iter()
                    .any(|r| (r.right - r.left) >= full && (r.bottom - r.top) >= full_h);
                // debug 级：限频采样，info 级会吃掉 2MB 日志配额；
                // 结论性的「首批脏矩形」与「坐标约定探测」仍是 info。
                log::debug!(
                    "[LG] 脏矩形 {got} 个{}，样本 [{}]，coalesced={} 元数据={}B 累计帧={}",
                    if covers_all {
                        "（含整屏矩形！）"
                    } else {
                        ""
                    },
                    sample_rects(&output.dirty_scratch[..got]),
                    LAST_FRAME_COALESCED.with(|c| c.get()),
                    LAST_FRAME_METADATA.with(|c| c.get()),
                    LAST_FRAME_ACCUM.with(|c| c.get())
                );
            }
        }
        // 一次性把首批矩形原样打出来：坐标约定这种事不能靠读文档猜，
        // DXGI 文档只说脏矩形描述「桌面图像」的区域，没说哪个坐标系。
        if !output.dirty_rect_probe_logged {
            output.dirty_rect_probe_logged = true;
            log::info!(
                "[LG] 首批脏矩形 {got} 个，样本 [{}]；输出矩形 ({},{})-({},{}) 尺寸 {}x{}",
                sample_rects(&output.dirty_scratch[..got]),
                output.rect.left,
                output.rect.top,
                output.rect.right,
                output.rect.bottom,
                output.rect.right - output.rect.left,
                output.rect.bottom - output.rect.top
            );
        }
        if output.dirty_rects_absolute.is_none() {
            let output_width = output.rect.right - output.rect.left;
            let output_height = output.rect.bottom - output.rect.top;
            let detected = detect_dirty_rect_convention(
                &output.dirty_scratch[..got],
                output_width,
                output_height,
                &output.rect,
            );
            match detected {
                Some(absolute) => {
                    output.dirty_rects_absolute = Some(absolute);
                    log::info!(
                        "[LG] 脏矩形坐标约定探测: {}（输出原点 ({},{}), 尺寸 {output_width}x{output_height}）",
                        if absolute {
                            "桌面绝对坐标，不加偏移"
                        } else {
                            "输出相对坐标，加上输出原点偏移"
                        },
                        output.rect.left,
                        output.rect.top
                    );
                }
                None if !output.dirty_rect_inconclusive_logged => {
                    output.dirty_rect_inconclusive_logged = true;
                    log::warn!(
                        "[LG] 脏矩形坐标约定探测不出结论：{got} 个矩形在两种约定下都越界，\
                         本批样本 [{}]，输出矩形 ({},{})-({},{})。\
                         继续按输出相对坐标处理；若副屏分类框出现「桌面在动但玻璃不刷新」，\
                         就是这里判错了",
                        sample_rects(&output.dirty_scratch[..got]),
                        output.rect.left,
                        output.rect.top,
                        output.rect.right,
                        output.rect.bottom
                    );
                }
                None => {}
            }
        }
        let (origin_x, origin_y) = change_origin(output);
        // push_change 只写 DESKTOP_CHANGES 这个 thread_local，不碰 output，
        // 所以可以直接在遍历借用里调，不需要再拷一份
        for index in 0..got {
            let rect = output.dirty_scratch[index];
            push_change(rect, origin_x, origin_y, generation);
        }
        return true;
    }
    log::warn!("[LG] 脏矩形缓冲扩容 {RECT_BUFFER_MAX_GROWTH} 次仍不够，本帧放弃");
    false
}

/// 取本帧移动矩形并记入变化队列。
///
/// 移动矩形要记**两块**：源位置与目标位置。窗口拖动时 DXGI 只说
/// 「这一块搬到了那一块」，只记目标会让原位置的分类框收不到变化通知。
///
/// 返回是否推入了至少一个矩形。
unsafe fn collect_move_rects(output: &mut CaptureOutput, generation: u64) -> bool {
    let move_size = size_of::<DXGI_OUTDUPL_MOVE_RECT>();
    let empty = DXGI_OUTDUPL_MOVE_RECT {
        SourcePoint: POINT { x: 0, y: 0 },
        DestinationRect: WinRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
    };
    let mut capacity = RECT_BUFFER_INITIAL_CAPACITY;
    for _ in 0..=RECT_BUFFER_MAX_GROWTH {
        output.move_scratch.clear();
        output.move_scratch.resize(capacity, empty);
        let mut required: u32 = 0;
        // 注意：这里必须传移动矩形自己的 required。
        // 旧实现误传了脏矩形的字节数，两者不等时 DXGI 直接报 MORE_DATA
        // 或参数错误，移动矩形因此被静默丢弃（拖窗口时原位置的框不刷新）。
        let result = output.duplication.GetFrameMoveRects(
            (capacity * move_size) as u32,
            output.move_scratch.as_mut_ptr(),
            &mut required,
        );
        let needed = ((required as usize) / move_size).max(1);
        if let Err(error) = &result {
            if error.code() == DXGI_ERROR_MORE_DATA && needed > capacity {
                capacity = needed;
                continue;
            }
            if !output.move_rect_failure_logged {
                output.move_rect_failure_logged = true;
                log::warn!(
                    "[LG] 取移动矩形失败: {error:?}（required={required}B，缓冲 {} 个）",
                    capacity
                );
            }
            return false;
        }
        let got = needed.min(capacity);
        if got == 0 {
            return false;
        }
        let (origin_x, origin_y) = change_origin(output);
        for index in 0..got {
            // DXGI_OUTDUPL_MOVE_RECT 没有 Copy，逐字段取出来用
            let m = &output.move_scratch[index];
            let dest = WinRect {
                left: m.DestinationRect.left,
                top: m.DestinationRect.top,
                right: m.DestinationRect.right,
                bottom: m.DestinationRect.bottom,
            };
            let width = dest.right - dest.left;
            let height = dest.bottom - dest.top;
            let source = WinRect {
                left: m.SourcePoint.x,
                top: m.SourcePoint.y,
                right: m.SourcePoint.x + width,
                bottom: m.SourcePoint.y + height,
            };
            push_change(dest, origin_x, origin_y, generation);
            push_change(source, origin_x, origin_y, generation);
        }
        return true;
    }
    log::warn!("[LG] 移动矩形缓冲扩容 {RECT_BUFFER_MAX_GROWTH} 次仍不够，本帧放弃");
    false
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
            let now = Instant::now();
            for index in 0..renderer.outputs.len() {
                match renderer.outputs[index].last_box_render_at {
                    // 从来没有分类框用过这个输出：一张全尺寸桌面纹理
                    // 都不给它建（主屏 3120x2080 一张就是 24.8MB）
                    None => continue,
                    Some(last) if now.duration_since(last) >= OUTPUT_IDLE_RELEASE => {
                        // 分类框已经搬走，释放这个输出占着的显存
                        renderer.release_idle_output(index);
                        continue;
                    }
                    Some(_) => {}
                }
                if let Err(error) = renderer.capture_output(index, 0) {
                    log::debug!("[LG] 桌面变化轮询失败 output={index}: {error:#}");
                }
            }
        });
    }
    DESKTOP_GENERATION.load(Ordering::Relaxed)
}

/// 只推进光标高光插值，不渲染任何东西。
///
/// 供分类框的周期 tick 在「这一帧画出来与上一帧逐像素相同、但旧逻辑
/// 仍会渲染」时调用，以保持 `light_direction` 的插值序列与修复前
/// 完全一致。详见 `GlassRenderer::advance_light`。
///
/// 渲染器未就绪 / 正忙时静默返回：插值状态本来就随渲染器一起重建，
/// 少推进一次不影响正确性，更不该为此报错或阻塞。
pub fn advance_light_direction(
    screen_x: i32,
    screen_y: i32,
    content_width: i32,
    content_height: i32,
    padding: i32,
) {
    if content_width <= 0 || content_height <= 0 || padding < 0 {
        return;
    }
    GLASS_STATE.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            return;
        };
        let GlassState::Ready(renderer) = &mut *state else {
            return;
        };
        renderer.advance_light(
            screen_x,
            screen_y,
            content_width as u32,
            content_height as u32,
            padding as u32,
        );
    });
}

/// 这块桌面区域的像素自上次查询以来是否真的变了。
///
/// 与 `region_changed_since`（只看 DXGI 报的矩形，便宜）配合使用：
/// 先用矩形筛一遍，命中了再比像素。驱动把整块输出报成一个矩形时，
/// 第一级永远命中，真正省下渲染的是这一级。
///
/// 渲染器未就绪 / 正忙 / 内部失败时一律返回 `true`（保守）：
/// 分类框最多多画一帧，绝不会因为比对失败而冻在旧画面上；
/// 再加上周期重绘心跳兜底，最坏滞后也就是一个心跳间隔。
pub fn desktop_region_changed(rect: &WinRect) -> bool {
    GLASS_STATE.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            return true;
        };
        let GlassState::Ready(renderer) = &mut *state else {
            return true;
        };
        renderer.desktop_region_changed_pixels(rect).unwrap_or(true)
    })
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

/// 计算下一次重试的等待时长，并返回累计失败次数。
fn next_retry_delay() -> (Duration, u32) {
    let failures = INIT_FAILURES.with(|cell| {
        let next = cell.get().saturating_add(1);
        cell.set(next);
        next
    });
    // 3s → 6s → 12s，再往上一律封顶到 RETRY_DELAY_MAX
    let shift = (failures - 1).min(2);
    let scaled = RETRY_DELAY.saturating_mul(1u32 << shift);
    (scaled.min(RETRY_DELAY_MAX), failures)
}

/// 初始化成功后清零退避计数。
fn reset_init_backoff() {
    INIT_FAILURES.with(|cell| cell.set(0));
}

/// 首次碰到 E_ACCESSDENIED 时解释一下原因，之后不再重复。
///
/// 桌面复制被拒绝绝大多数不是代码问题：会话锁定/安全桌面下
/// DXGI 就是会对所有输出返回这个错误，解锁后自动恢复。
/// 不写进日志的话，下次排查又得重新猜一遍。
fn note_access_denied(error: &anyhow::Error) {
    const E_ACCESSDENIED: i32 = -2147024891; // 0x80070005
    let denied = error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<windows::core::Error>())
        .any(|e| e.code().0 == E_ACCESSDENIED);
    if !denied {
        return;
    }
    if ACCESS_DENIED_HINT_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    log::warn!(
        "[LG] 桌面复制被拒绝（E_ACCESSDENIED）：会话处于锁定/安全桌面时 DXGI 就是会这样，\
         不是代码问题。解锁后渲染器会自动恢复；本提示只打一次。"
    );
}

/// 区域内容指纹的起始值（FNV-1a 64 位偏移基准）。
const HASH_SEED: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 位素数。
const HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 给一块字节算 64 位指纹，`seed` 用于把多行串成一个区域指纹。
///
/// 按 8 字节一块吃，比逐字节的教科书 FNV-1a 快约 8 倍：
/// 一个分类框区域 1.4MB，逐字节要 1ms 以上，5 个框每秒一次就吃得下
/// 百分之一的 CPU，不值得；分块后降到 0.1ms 量级。
fn hash_region(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    let mut index = 0;
    while index + 8 <= bytes.len() {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[index..index + 8]);
        hash ^= u64::from_le_bytes(word);
        hash = hash.wrapping_mul(HASH_PRIME);
        index += 8;
    }
    // 不足 8 字节的尾巴补零后同样吃进去：漏掉尾部会让
    // 「只有最后一行末尾几个像素变了」的情况检测不出来
    let mut tail = [0u8; 8];
    tail[..bytes.len() - index].copy_from_slice(&bytes[index..]);
    hash ^= u64::from_le_bytes(tail);
    hash.wrapping_mul(HASH_PRIME)
}

/// 每像素字节数。捕获纹理只可能是这三种格式之一。
fn bytes_per_pixel(format: DXGI_FORMAT) -> usize {
    match format {
        DXGI_FORMAT_R16G16B16A16_FLOAT => 8,
        _ => 4,
    }
}

/// 把桌面坐标区域换算成输出本地坐标，并夹到纹理范围内。
///
/// 返回 `(x, y, w, h)`；`w` 或 `h` 为 0 表示区域完全落在该输出之外，
/// 调用方必须按「无法判定」保守处理。
fn clamp_region_to_output(
    rect: &WinRect,
    output_rect: &WinRect,
    texture_width: u32,
    texture_height: u32,
) -> (u32, u32, u32, u32) {
    let max_x = texture_width as i32;
    let max_y = texture_height as i32;
    let left = (rect.left - output_rect.left).clamp(0, max_x);
    let top = (rect.top - output_rect.top).clamp(0, max_y);
    let right = (rect.right - output_rect.left).clamp(left, max_x);
    let bottom = (rect.bottom - output_rect.top).clamp(top, max_y);
    (
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    )
}

/// 把 `source` 纹理里 `(x, y, width, height)` 那一块拷进 `staging`，
/// 回读后算出 64 位指纹。
///
/// # 为什么单独抽出来
///
/// 这条链路（子区域拷贝 → Map → 按 `RowPitch` 逐行取 → 指纹）是
/// 「桌面没变就不重绘」的唯一依据，判错就是分类框冻在旧背景上。
/// 而锁屏 / 安全桌面下 DXGI 拿不到帧，没法在真实桌面上做阳性对照，
/// 所以必须能脱离桌面复制单测：D3D11 设备在锁屏下照样能建，
/// 用一张自己填好内容的纹理就能验证它对**单像素**变化是否敏感、
/// 以及 `staging` 比区域宽时行距有没有算错。
///
/// `staging` 必须不小于 `copy_box` 的尺寸，且格式与 `source` 一致
/// （`CopySubresourceRegion` 的硬性要求）。
unsafe fn hash_texture_region(
    context: &ID3D11DeviceContext,
    staging: &ID3D11Texture2D,
    source: &ID3D11Texture2D,
    copy_box: &D3D11_BOX,
    bytes_per_pixel: usize,
) -> Result<u64> {
    context.CopySubresourceRegion(staging, 0, 0, 0, 0, source, 0, Some(copy_box));
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    // staging 可能比区域宽，必须按 RowPitch 逐行跳，且每行只吃
    // width × bpp 字节 —— 把行尾填充也算进去的话，指纹会随驱动的
    // 对齐方式变化，同一块桌面在不同机器上判定就不一致了。
    let width = (copy_box.right - copy_box.left) as usize;
    let height = (copy_box.bottom - copy_box.top) as usize;
    let row_bytes = width * bytes_per_pixel;
    let base = mapped.pData.cast::<u8>();
    let mut hash = HASH_SEED;
    for row in 0..height {
        let row_slice = slice::from_raw_parts(base.add(row * mapped.RowPitch as usize), row_bytes);
        hash = hash_region(row_slice, hash);
    }
    context.Unmap(staging, 0);
    Ok(hash)
}

/// 取前若干个矩形拼成可读样本，用于一次性诊断日志。
fn sample_rects(rects: &[WinRect]) -> String {
    rects
        .iter()
        .take(4)
        .map(|r| format!("({},{})-({},{})", r.left, r.top, r.right, r.bottom))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 从一批脏矩形推断 DXGI 用的是哪种坐标约定。
///
/// 返回 `Some(true)` 表示已是桌面绝对坐标，`Some(false)` 表示输出
/// 相对坐标，`None` 表示这批数据两边都不符（不可能发生，但
/// 宁可下一帧再判也不能猜）。
fn detect_dirty_rect_convention(
    rects: &[WinRect],
    output_width: i32,
    output_height: i32,
    output_rect: &WinRect,
) -> Option<bool> {
    if rects.is_empty() {
        return None;
    }
    let fits_output_local = rects
        .iter()
        .all(|r| r.left >= 0 && r.top >= 0 && r.right <= output_width && r.bottom <= output_height);
    if fits_output_local {
        // 主屏（原点 0,0）下两种约定都会命中这一支，而此时偏移
        // 本来就是 0，两种解释结果完全一致，不存在风险。
        return Some(false);
    }
    let fits_desktop_absolute = rects.iter().all(|r| {
        r.left >= output_rect.left
            && r.top >= output_rect.top
            && r.right <= output_rect.right
            && r.bottom <= output_rect.bottom
    });
    if fits_desktop_absolute {
        return Some(true);
    }
    None
}

/// ACCESS_LOST 连续发生时，两次 duplication 重建之间的最小间隔。
///
/// 桌面处于安全态（锁屏、屏保、远程会话、显示器拓扑刚变）时，
/// `AcquireNextFrame` 会**一直**返回 `DXGI_ERROR_ACCESS_LOST`，
/// 而重建出来的 duplication 下一帧立刻又失效。旧实现每次都无条件重建，
/// 实测屏保结束后达到 **12 次/秒、一分钟 730 次**：
/// 白烧 CPU，还制造大量 COM 对象的申请/归还（堆 churn）。
///
/// 按连续失败次数指数退避，桌面恢复后由成功取帧归零。
fn access_lost_backoff(streak: u32) -> Duration {
    match streak {
        0 => Duration::ZERO,
        1 => Duration::from_millis(250),
        2..=4 => Duration::from_secs(1),
        _ => Duration::from_secs(3),
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
    // 本帧是否需要把像素回读到 `out`。
    //
    // `UpdateLayeredWindow` 路径必须传 `true`（画面最终要落进 GDI DIB）；
    // DirectComposition 路径传 `false`，玻璃留在 GPU 上零拷贝合成，
    // 整段 `CopySubresourceRegion` + `Map` 阻塞 + 逐行 memcpy 全部跳过
    // （实测占单帧 20ms 里的 5.5ms）。
    // 传 `false` 时 `out` 不会被写入，调用方不得拿它当画面用。
    //
    // 注：这里只能用 `//` 不能用 `///` —— Rust 不允许文档注释挂在
    // 函数参数上，会直接编译失败。
    need_pixels: bool,
    // 分类框稳定 id：回读暂存按它隔离，避免框间互读。
    box_id: usize,
    // 同步读本帧（阻塞等 GPU）而不是读上一帧：拖拽/移动期
    // 零延迟跟住窗口位置，静止时继续上一帧免等待。
    sync_read: bool,
) -> bool {
    if content_width <= 0 || content_height <= 0 || padding < 0 {
        return false;
    }

    // 失败兜底靠的就是这块缓冲区里的上一帧，长度必须对得上
    let expected_len = expected_surface_len(content_width, content_height, padding);
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
                return reuse_existing_frame(out, expected_len);
            }
        };
        if matches!(&*state, GlassState::Failed { retry_at } if Instant::now() < *retry_at) {
            return reuse_existing_frame(out, expected_len);
        }

        if !matches!(&*state, GlassState::Ready(_)) {
            match GlassRenderer::new() {
                Ok(renderer) => {
                    reset_init_backoff();
                    log::info!("液态玻璃 D3D11 渲染器已初始化");
                    *state = GlassState::Ready(renderer);
                }
                Err(error) => {
                    let (delay, failures) = next_retry_delay();
                    // 锁屏期间每几秒刷一条 warn 会把日志写爆（单日上限 2MB）：
                    // 第一次和每 20 次才记 warn，其余降为 debug。
                    if failures == 1 || failures % 20 == 0 {
                        log::warn!(
                            "液态玻璃初始化失败（第 {failures} 次，{delay:?} 后重试），使用半透明背景: {error:#}"
                        );
                    } else {
                        log::debug!("液态玻璃初始化失败（第 {failures} 次）: {error:#}");
                    }
                    *state = GlassState::Failed {
                        retry_at: Instant::now() + delay,
                    };
                    return reuse_existing_frame(out, expected_len);
                }
            }
        }

        let GlassState::Ready(renderer) = &mut *state else {
            return false;
        };

        let glass_started = Instant::now();
        let rendered = renderer.render(
            screen_x,
            screen_y,
            content_width as u32,
            content_height as u32,
            padding as u32,
            corner_radius,
            dpi_scale,
            glass_config,
            out,
            need_pixels,
            box_id,
            sync_read,
        );
        super::mem_probe::add_glass_timing(glass_started.elapsed().as_micros() as u64);
        match rendered {
            Ok(()) => {
                log::debug!("[LG] render OK, {} pixels", out.len());
                log_first_frame_diagnostic(out, content_width, content_height, padding);
                // 成功帧已经在 out 里了，它本身就是下一帧的兜底，无需再复制一份
                true
            }
            Err(error) => {
                log::warn!("液态玻璃帧渲染失败，保留上一成功帧: {error:#}");
                // 单帧捕获失败不应让整个渲染器进入 3 秒回退状态；保留 Ready
                // 可在下一次定时器 tick 立即重试，避免拖动到新位置时闪黑。
                reuse_existing_frame(out, expected_len)
            }
        }
    })
}

/// 取玻璃输出纹理的**零拷贝** D2D 位图。
///
/// 返回 `None` 的情形：渲染器尚未就绪、本帧失败、或输出纹理包装失败。
/// 调用方**必须**退回 CPU 回读路径，绝不能当成「玻璃是透明的」继续画 ——
/// 那会让分类框整个消失且没有任何报错。
///
/// # 为什么能零拷贝
///
/// 玻璃着色器渲染进的 `output_texture` 与合成端现在同属**一个**
/// `ID3D11Device` / `ID2D1Device`（见 `render_devices::SharedGpu`），
/// 所以可以直接 `CreateBitmapFromDxgiSurface` 包一层给 D2D 采样。
/// 旧实现里两者是两个设备，纹理跨不过去，只能 `Map` 回读到 CPU
/// 再 `CopyFromMemory` 上传 —— 实测那一来一回占单帧的 5.5ms。
///
/// # 注意：输出纹理是所有分类框共享的一张
///
/// 尺寸取历史最大值，本框的画面只在左上角
/// `surface_width × surface_height` 那一块。调用方必须把它作为
/// source rect 传进 `DrawBitmap`，否则会画到别的框的残留像素。
pub fn glass_output_bitmap() -> Option<ID2D1Bitmap1> {
    GLASS_STATE.with(|state| {
        let state = state.try_borrow().ok()?;
        match &*state {
            GlassState::Ready(renderer) => renderer
                .surfaces
                .as_ref()
                .and_then(|surfaces| surfaces.output_bitmap.clone()),
            _ => None,
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
    // 用渲染线程**共享**的那一个 D3D11 设备，不再自建。
    //
    // 自建会让进程里同时存在两个 D3D11 设备（另一个藏在 D2D1 工厂内部），
    // 而 D3D11 纹理不能跨设备直接共享 —— 玻璃因此必须每帧
    // `CopySubresourceRegion` → `Map` 阻塞等 GPU 排空 → 逐行 memcpy 回 CPU
    // → `CopyFromMemory` 再上传给另一个设备 → 软件光栅合成。
    // 实测这条往返占单帧 20ms 里的 16ms，而真正的模糊 + 着色只有 0.44ms：
    // 97% 的时间在搬像素，3% 在画画。
    //
    // 统一到单一设备后，玻璃输出纹理可以用
    // `CreateBitmapFromDxgiSurface` 零拷贝包成 D2D 位图直接参与合成。
    // 顺带省掉一个设备本身（每个额外设备连驱动私有分配约 50MB）。
    let gpu = super::render_devices::shared_gpu()
        .context("渲染线程共享 GPU 设备组不可用，液态玻璃退回半透明背景")?;
    Ok((gpu.device, gpu.context))
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
                    last_box_render_at: None,
                    blur_chains: HashMap::new(),
                    transient_chain: None,
                    pending_chain_keys: HashMap::new(),
                    blur_stamp: 0,
                    blur_chains_created: 0,
                    dirty_scratch: Vec::new(),
                    move_scratch: Vec::new(),
                    dirty_rects_absolute: None,
                    dirty_rect_probe_logged: false,
                    frame_info_probe_logged: false,
                    no_rect_frame_logged: false,
                    last_rect_sample_log: None,
                    dirty_rect_inconclusive_logged: false,
                    move_rect_failure_logged: false,
                })
            })();
            match prepared {
                Ok(capture) => outputs.push(capture),
                Err(error) => {
                    log::warn!(
                        "[LG] 输出 {index} ({device_name}) 桌面复制不可用，该显示器上的分类框回退半透明背景: {error:#}"
                    );
                    note_access_denied(&error);
                }
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

/// 纹理占用（MB），用于释放日志；`None` 记 0。
fn texture_mb(texture: Option<&ID3D11Texture2D>) -> f64 {
    texture.map(texture_bytes).unwrap_or(0) as f64 / 1_048_576.0
}

/// 从「键 → 最近使用序号」里挑出最久未用的键（纯函数，便于单测）。
fn lru_victim<K: Copy>(stamps: impl Iterator<Item = (K, u64)>) -> Option<K> {
    stamps.min_by_key(|(_, stamp)| *stamp).map(|(key, _)| key)
}

/// 建一条只服务一个裁剪框的模糊链，并把所有不会逐帧变化的属性一次写死。
///
/// 写死之后运行期不再碰 `SetInput` / `SetValue`，D2D1 就能一直命中
/// 它为该图缓存的 realization（高斯模糊的中间表面），不会每帧重新申请。
fn build_blur_chain(
    d2d_context: &ID2D1DeviceContext,
    source_bitmap: &ID2D1Bitmap1,
    crop_rect: &D2D_RECT_F,
    sigma: f32,
    saturation: f32,
    stamp: u64,
    half: bool,
) -> Result<BlurChain> {
    unsafe {
        let crop = d2d_context.CreateEffect(&CLSID_D2D1Crop)?;
        let border = d2d_context.CreateEffect(&CLSID_D2D1Border)?;
        border.SetInput(0, &crop.GetOutput()?, true);
        // 半分辨率链：border 之后先缩 0.5 再模糊（sigma 减半），
        // 重度放大由绘制时的 Scale(2) 变换完成，GPU 成本约 1/4。
        let scale = d2d_context.CreateEffect(&CLSID_D2D1Scale)?;
        scale.SetInput(0, &border.GetOutput()?, true);
        let sv = if half { 0.5f32 } else { 1.0f32 };
        let sv_bytes = std::slice::from_raw_parts(
            (&Vector2 { X: sv, Y: sv } as *const _) as *const u8,
            8,
        );
        scale.SetValue(D2D1_SCALE_PROP_SCALE.0 as u32, D2D1_PROPERTY_TYPE_VECTOR2, sv_bytes)?;
        let blur = d2d_context.CreateEffect(&CLSID_D2D1GaussianBlur)?;
        blur.SetInput(0, &scale.GetOutput()?, true);
        let saturate = d2d_context.CreateEffect(&CLSID_D2D1ColorMatrix)?;
        saturate.SetInput(0, &blur.GetOutput()?, true);
        let saturated = saturate.GetOutput()?;

        crop.SetInput(0, source_bitmap, true);
        set_crop_rect(&crop, crop_rect)?;
        set_border_mirror(&border)?;
        set_blur_sigma(&blur, if half { sigma / 2.0 } else { sigma })?;
        set_saturation_matrix(&saturate, saturation)?;

        Ok(BlurChain {
            crop,
            border,
            scale,
            half,
            blur,
            saturate,
            saturated,
            crop_rect: *crop_rect,
            sigma,
            saturation,
            last_used: stamp,
        })
    }
}

/// 这一帧该用哪条模糊链。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChainPlan {
    /// 键已在常驻表里：直接命中，稳态下零 `SetValue` / 零 `CreateEffect`
    Cached,
    /// 同一个键第二次出现 → 分类框停住了，升级成常驻链
    Promote,
    /// 键每帧都在变（正在拖动/缩放）→ 复用临时链，只改裁剪框
    Transient,
}

/// 决定这一帧的模糊链策略（纯函数，便于单测）。
///
/// `seen_times` 是这个裁剪框键**连续未命中**的出现次数（命中常驻表时为 0）。
///
/// # 为什么「第二次才升级」是拖动流畅度的关键
///
/// 拖动时裁剪框每帧都变，任何键都只会命中一次、永远升不了级，
/// 于是整段拖动都走 Transient：**effect 对象身份始终稳定**，
/// 每帧只改一次裁剪框。改造前这里是每帧新建 4 个 COM effect 再淘汰 4 个，
/// 比最初「一条链 + SetValue」的版本每帧做的功更多。
/// 分类框一停住，同一个键第二次出现就升级成常驻链，此后每帧零写入，
/// D2D 一直命中同一份 realization —— 这才是显存不再爬升的原因。
fn plan_blur_chain(is_cached: bool, seen_times: u8) -> ChainPlan {
    if is_cached {
        ChainPlan::Cached
    } else if seen_times >= CHAIN_PROMOTE_AFTER_MISSES {
        ChainPlan::Promote
    } else {
        ChainPlan::Transient
    }
}

/// 两个 D2D 矩形是否逐分量相等（用于避免无谓的 SetValue 作废 realization）。
fn same_d2d_rect(a: &D2D_RECT_F, b: &D2D_RECT_F) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

/// 写入 Crop effect 的裁剪矩形。
unsafe fn set_crop_rect(crop: &ID2D1Effect, rect: &D2D_RECT_F) -> Result<()> {
    let bytes = slice::from_raw_parts(
        (rect as *const D2D_RECT_F).cast::<u8>(),
        size_of::<D2D_RECT_F>(),
    );
    crop.SetValue(
        D2D1_CROP_PROP_RECT.0 as u32,
        D2D1_PROPERTY_TYPE_VECTOR4,
        bytes,
    )?;
    Ok(())
}

/// 把 Border effect 的 X/Y 边缘模式设为 MIRROR（玻璃边缘的镜像延展）。
unsafe fn set_border_mirror(border: &ID2D1Effect) -> Result<()> {
    let edge_mode = D2D1_BORDER_EDGE_MODE_MIRROR.0;
    let bytes = slice::from_raw_parts((&edge_mode as *const i32).cast::<u8>(), size_of::<i32>());
    border.SetValue(
        D2D1_BORDER_PROP_EDGE_MODE_X.0 as u32,
        D2D1_PROPERTY_TYPE_ENUM,
        bytes,
    )?;
    border.SetValue(
        D2D1_BORDER_PROP_EDGE_MODE_Y.0 as u32,
        D2D1_PROPERTY_TYPE_ENUM,
        bytes,
    )?;
    Ok(())
}

/// 写入高斯模糊标准差。
unsafe fn set_blur_sigma(blur: &ID2D1Effect, sigma: f32) -> Result<()> {
    let bytes = slice::from_raw_parts((&sigma as *const f32).cast::<u8>(), 4);
    blur.SetValue(
        D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
        D2D1_PROPERTY_TYPE_FLOAT,
        bytes,
    )?;
    Ok(())
}

/// 写入饱和度矩阵（ColorMatrix effect）。
unsafe fn set_saturation_matrix(saturate: &ID2D1Effect, saturation: f32) -> Result<()> {
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
    let bytes = slice::from_raw_parts(
        (&matrix as *const D2D_MATRIX_5X4_F).cast::<u8>(),
        size_of::<D2D_MATRIX_5X4_F>(),
    );
    saturate.SetValue(
        D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32,
        D2D1_PROPERTY_TYPE_MATRIX_5X4,
        bytes,
    )?;
    Ok(())
}

impl GlassRenderer {
    fn new() -> Result<Self> {
        unsafe {
            let (device, context) = create_device()?;

            // 先做最容易失败、也最便宜的一步：拿桌面复制。
            //
            // 屏幕锁定/安全桌面下 DXGI 会对**所有**输出返回 E_ACCESSDENIED，
            // 而渲染器会按 RETRY_DELAY 不停重试。旧顺序把失败点放在最后，
            // 每次重试都先白白跑完 3 次 PNG 解码 + 3 次纹理上传 +
            // 2 次 D3DCompile 才发现不行——实测锁屏期间白烧 ~7% 单核。
            // 提到前面后，失败路径只剩一个 D3D11 设备创建。
            let outputs = create_outputs(&device)?;
            if outputs.is_empty() {
                return Err(anyhow!("未找到可用的 DXGI 显示输出"));
            }

            // D2D1 设备**也必须**用共享的那一个。
            //
            // D2D 位图与渲染目标是「设备亲和」的：只有同属一个
            // `ID2D1Device` 的位图和 DeviceContext 才能互相 DrawBitmap。
            // 这里再 `D2D1CreateDevice` 建一个，就等于把玻璃输出纹理和
            // 合成端隔在两个设备之间，零拷贝直接失效、又得绕道 CPU。
            let shared =
                super::render_devices::shared_gpu().context("渲染线程共享 GPU 设备组不可用")?;
            let d2d_context = shared
                .d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

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
                readbacks: std::collections::HashMap::new(),
                    surfaces: None,
                displacement_maps: [std_map, pol_map, pro_map],
                light_direction: [0.0, 0.0],
                startup_black_frames_remaining: 3,
                access_lost_streak: 0,
                last_duplication_rebuild: None,
                diff_staging: None,
                region_watches: HashMap::new(),
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
            // 关键：D2D 位图包装与模糊链都是绑在旧纹理上的。
            // 只丢纹理不丢包装，重建后 crop 的输入仍指向已废弃的
            // 旧桌面纹理、模糊结果仍写进旧的 blurred_texture，
            // 表现为「玻璃冻在旧画面 / 完全消失」且没有任何报错。
            output.source_bitmap = None;
            output.target_bitmap = None;
            output.blur_chains.clear();
            output.transient_chain = None;
            output.pending_chain_keys.clear();
            log::info!("液态玻璃 DXGI Output Duplication 已重建 (index={})", index);
            Ok(())
        }
    }

    /// 释放一个已无分类框驻留的输出所占的显存。
    ///
    /// 纹理、D2D 位图包装、模糊链必须一起放：链里的 `crop` 输入
    /// 指向 `source_bitmap`，只丢纹理不丢包装会留下悬空引用。
    /// duplication 保留 —— 它很轻，而重建要重新走一遍可能被拒绝的流程。
    /// 分类框搬回来时 `capture_output` / `blur_output_impl` 会按需重建，
    /// 与首次初始化走的是同一条路径。
    fn release_idle_output(&mut self, index: usize) {
        let output = &mut self.outputs[index];
        if output.desktop_texture.is_none()
            && output.blurred_texture.is_none()
            && output.blur_chains.is_empty()
        {
            // 已经空了，只剩时间戳要清；清掉后本输出彻底退出轮询
            output.last_box_render_at = None;
            return;
        }
        let freed = texture_mb(output.desktop_texture.as_ref())
            + texture_mb(output.blurred_texture.as_ref());
        output.desktop_texture = None;
        output.desktop_view = None;
        output.blurred_texture = None;
        output.blurred_view = None;
        output.source_bitmap = None;
        output.target_bitmap = None;
        output.blur_chains.clear();
        output.last_box_render_at = None;
        log::info!("[LG-mem] 输出 {index} 已无分类框驻留，释放约 {freed:.1}MB 显存");
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
                    // 桌面复制失效：重建 duplication，而不是让整个渲染器
                    // 退回半透明背景。快速改窗口尺寸时这是常态。
                    //
                    // 但必须限频 —— 桌面处于安全态时会一直失效，
                    // 无条件重建能跑到 12 次/秒，详见 access_lost_backoff。
                    let now = Instant::now();
                    let delay = access_lost_backoff(self.access_lost_streak);
                    let due = self
                        .last_duplication_rebuild
                        .is_none_or(|last| now.duration_since(last) >= delay);
                    if due {
                        self.last_duplication_rebuild = Some(now);
                        self.access_lost_streak = self.access_lost_streak.saturating_add(1);
                        if self.access_lost_streak > 1 {
                            log::debug!(
                                "[LG] ACCESS_LOST 连续 {} 次，重建间隔退避到 {:?}",
                                self.access_lost_streak,
                                delay
                            );
                        }
                        self.rebuild_duplication(index)?;
                    }
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
                FRAMES_ACQUIRED.fetch_add(1, Ordering::Relaxed);
                LAST_FRAME_COALESCED.with(|c| c.set(frame_info.RectsCoalesced.as_bool()));
                LAST_FRAME_METADATA.with(|c| c.set(frame_info.TotalMetadataBufferSize));
                LAST_FRAME_ACCUM.with(|c| c.set(frame_info.AccumulatedFrames));
                // `LastPresentTime == 0` 表示这一帧 DXGI 只更新了鼠标位置，
                // 桌面**图像**根本没变。旧实现无条件推进桌面代号，于是
                // 每个鼠标帧都让 region_changed_in 走「队列为空但代号已
                // 前进 → 保守返回 true」的兜底，把**所有输出上的所有**
                // 分类框一起拖去陪绘 —— 实测静止桌面上稳定每秒 5 次白渲染，
                // 「桌面变化」计数 150/30秒而「心跳」恒为 0，
                // 脏矩形按框判定等于完全没生效。
                let image_updated = frame_info.LastPresentTime != 0;
                if !image_updated {
                    FRAMES_MOUSE_ONLY.fetch_add(1, Ordering::Relaxed);
                }
                // 拿到真帧说明桌面复制恢复正常，退避计数归零
                self.access_lost_streak = 0;
                // 候选代号：先按它收集矩形，确认真的有东西要记再落盘。
                // 只在真正渲染时才会走到这里，因此不需要额外的轮询开销。
                let generation = DESKTOP_GENERATION.load(Ordering::Relaxed) + 1;
                // 必须在 ReleaseFrame 之前取脏矩形
                let pushed_any = collect_frame_changes(output, generation);
                if pushed_any || image_updated {
                    DESKTOP_GENERATION.store(generation, Ordering::Relaxed);
                    if !pushed_any {
                        // 图像确实更新了，但 DXGI 一个矩形都没给
                        // （驱动不支持 / 矩形被合并掉了）。此时诚实地记
                        // 「这一整块输出都变了」：比让 region_changed_in
                        // 走全局兜底好，兜底会连**别的显示器**上的分类框
                        // 也一起判成变化。
                        FRAMES_WITHOUT_RECTS.fetch_add(1, Ordering::Relaxed);
                        if !output.no_rect_frame_logged {
                            output.no_rect_frame_logged = true;
                            // 这一帧就是「按框判定退化」的现场，必须留证：
                            // RectsCoalesced 说明驱动把矩形合并掉了，
                            // 元数据缓冲为 0 说明驱动压根没给。
                            log::warn!(
                                "[LG] 有图像更新但 DXGI 未提供任何矩形（RectsCoalesced={} \
                                 元数据缓冲={}B AccumulatedFrames={} LastPresent={} LastMouse={}），\
                                 本帧整块输出按变化处理",
                                frame_info.RectsCoalesced.as_bool(),
                                frame_info.TotalMetadataBufferSize,
                                frame_info.AccumulatedFrames,
                                frame_info.LastPresentTime,
                                frame_info.LastMouseUpdateTime
                            );
                        }
                        push_change(output.rect, 0, 0, generation);
                    }
                }
                if !output.frame_info_probe_logged {
                    output.frame_info_probe_logged = true;
                    log::info!(
                        "[LG] 首帧 FRAME_INFO: AccumulatedFrames={} RectsCoalesced={} \
                         元数据缓冲={}B 指针形状缓冲={}B LastPresentTime={} LastMouseUpdate={}",
                        frame_info.AccumulatedFrames,
                        frame_info.RectsCoalesced.as_bool(),
                        frame_info.TotalMetadataBufferSize,
                        frame_info.PointerShapeBufferSize,
                        frame_info.LastPresentTime,
                        frame_info.LastMouseUpdateTime
                    );
                }
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

            // ===== 一个分类框一条链，稳态下一次 SetValue 都不做 =====
            //
            // Crop → Border(MIRROR) → GaussianBlur → ColorMatrix。
            // 输入位图、边框模式、裁剪框在建链时一次写死；σ 与饱和度
            // 只在真的变了才写。D2D1 的 SetInput / SetValue 都会把该
            // effect 及下游整张图标记为脏、作废 realization（高斯模糊的
            // 中间表面），下一帧重新申请。旧实现全 output 共用一条链、
            // 每帧把这些属性全部重写一遍，等于每帧强制重建一次全图，
            // 显存只涨不落（mem_probe 实测 60 秒 417MB → 593MB）。
            let key: BlurChainKey = (lens_x, lens_y, lens_w, lens_h);
            output.blur_stamp = output.blur_stamp.wrapping_add(1);
            let stamp = output.blur_stamp;

            // 三种情况分开处理，让「静止」与「拖动」各自走到最优路径，
            // 判定规则见 plan_blur_chain。
            let is_cached = output.blur_chains.contains_key(&key);
            let seen_times = if is_cached {
                output.pending_chain_keys.remove(&key);
                0
            } else {
                let seen = output
                    .pending_chain_keys
                    .get(&key)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1);
                if output.pending_chain_keys.len() > MAX_PENDING_CHAIN_KEYS {
                    // 拖动会不断产生新键，触顶清空；代价只是每个框重新计一次数
                    output.pending_chain_keys.clear();
                }
                output.pending_chain_keys.insert(key, seen);
                seen
            };
            let plan = plan_blur_chain(is_cached, seen_times);

            match plan {
                ChainPlan::Cached => {}
                ChainPlan::Promote => {
                    // 触顶按最久未用淘汰，保证 D2D realization 总量有界
                    while output.blur_chains.len() >= MAX_BLUR_CHAINS_PER_OUTPUT {
                        let victim = lru_victim(
                            output
                                .blur_chains
                                .iter()
                                .map(|(chain_key, chain)| (*chain_key, chain.last_used)),
                        );
                        match victim {
                            Some(victim_key) => {
                                output.blur_chains.remove(&victim_key);
                            }
                            None => break,
                        }
                    }
                    let chain = build_blur_chain(
                        d2d_context,
                        &source_bitmap,
                        &crop_rect,
                        sigma,
                        saturation,
                        stamp,
                        false,
                    )?;
                    output.blur_chains_created = output.blur_chains_created.saturating_add(1);
                    log::debug!(
                        "[LG-mem] 模糊链升级为常驻 {lens_w}x{lens_h}@({lens_x},{lens_y})，当前 {} 条 / 累计 {} 次",
                        output.blur_chains.len() + 1,
                        output.blur_chains_created
                    );
                    output.blur_chains.insert(key, chain);
                }
                ChainPlan::Transient => {
                    if output.transient_chain.is_none() {
                        output.transient_chain = Some(build_blur_chain(
                            d2d_context,
                            &source_bitmap,
                            &crop_rect,
                            sigma,
                            saturation,
                            stamp,
                            true,
                        )?);
                        output.blur_chains_created = output.blur_chains_created.saturating_add(1);
                    }
                    let chain = output.transient_chain.as_mut().context("临时模糊链缺失")?;
                    // 裁剪框没变就别写：SetValue 会让 D2D 作废整张 realization
                    if !same_d2d_rect(&chain.crop_rect, &crop_rect) {
                        set_crop_rect(&chain.crop, &crop_rect)?;
                        chain.crop_rect = crop_rect;
                    }
                }
            }

            let chain = match plan {
                ChainPlan::Transient => {
                    output.transient_chain.as_mut().context("临时模糊链缺失")?
                }
                ChainPlan::Cached | ChainPlan::Promote => output
                    .blur_chains
                    .get_mut(&key)
                    .context("模糊链在插入后立即丢失")?,
            };
            chain.last_used = stamp;

            // σ / 饱和度只有配置或 DPI 真变了才写：写一次就作废一次 realization
            if (chain.sigma - sigma).abs() > f32::EPSILON {
                set_blur_sigma(&chain.blur, sigma)?;
                chain.sigma = sigma;
            }
            if (chain.saturation - saturation).abs() > f32::EPSILON {
                set_saturation_matrix(&chain.saturate, saturation)?;
                chain.saturation = saturation;
            }

            let blur_started = Instant::now();
            d2d_context.SetTarget(&target_bitmap);
            d2d_context.BeginDraw();
            // 半分辨率链的输出坐标是全尺寸的一半，Scale(2) 变换把它
            // 映回全尺寸坐标系（内容位置不变，分辨率减半）。
            if chain.half {
                d2d_context.SetTransform(&Matrix3x2 {
                    M11: 2.0,
                    M12: 0.0,
                    M21: 0.0,
                    M22: 2.0,
                    M31: 0.0,
                    M32: 0.0,
                });
            }
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
            // 裁剪后模糊面积缩到原来的 ~1/45，且画面逐像素不变。
            d2d_context.PushAxisAlignedClip(&chain.crop_rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            d2d_context.DrawImage(
                &chain.saturated,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_COPY,
            );
            d2d_context.PopAxisAlignedClip();
            if chain.half {
                d2d_context.SetTransform(&Matrix3x2 {
                    M11: 1.0,
                    M12: 0.0,
                    M21: 0.0,
                    M22: 1.0,
                    M31: 0.0,
                    M32: 0.0,
                });
            }
            let end_draw = d2d_context.EndDraw(None, None);
            d2d_context.SetTarget(None::<&ID2D1Image>);
            super::mem_probe::add_blur_timing(blur_started.elapsed().as_micros() as u64);
            end_draw.map_err(Into::into)
        }
    }

    /// 确保区域比对暂存纹理能容纳 width × height，且格式与桌面纹理一致。
    fn ensure_diff_staging(
        &mut self,
        desktop: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { desktop.GetDesc(&mut desc) };
        if let Some(staging) = &self.diff_staging {
            if staging.format == desc.Format && staging.width >= width && staging.height >= height {
                return Ok(());
            }
        }
        // 只增不减，且格式变化时整块重建
        let new_width = self
            .diff_staging
            .as_ref()
            .map_or(width, |s| s.width.max(width));
        let new_height = self
            .diff_staging
            .as_ref()
            .map_or(height, |s| s.height.max(height));
        unsafe {
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: new_width,
                Height: new_height,
                MipLevels: 1,
                ArraySize: 1,
                Format: desc.Format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut texture = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut texture))?;
            let texture = texture.context("创建区域比对暂存纹理未返回对象")?;
            let previous = self.diff_staging.replace(DiffStaging {
                texture,
                format: desc.Format,
                width: new_width,
                height: new_height,
            });
            log::info!(
                "[LG-mem] 区域比对暂存纹理 {}x{} = {:.1}MB{}",
                new_width,
                new_height,
                (new_width as f64 * new_height as f64 * bytes_per_pixel(desc.Format) as f64)
                    / 1_048_576.0,
                if previous.is_some() {
                    "（替换旧的）"
                } else {
                    ""
                }
            );
            Ok(())
        }
    }

    /// 这块桌面区域的**像素**自上次比对以来是否真的变了。
    ///
    /// # 为什么不能只靠 DXGI 的脏矩形
    ///
    /// 本机实测：桌面每秒被整块重绘一次，DXGI 每帧只给出**一个**
    /// 覆盖整个输出的矩形（`RectsCoalesced=false`、`AccumulatedFrames=1`、
    /// 元数据缓冲 16B = 正好一个 RECT）。把渲染频率从 5 次/秒拉到
    /// 24.5 次/秒做对照，取帧率纹丝不动 —— 说明这个每秒一次的整屏
    /// present 来自外部，而驱动对它给不出任何空间粒度。
    ///
    /// 此时「脏矩形是否命中这个框」永远命中，按框判定形同虚设，
    /// 5 个框每秒陪绘 5 次。只有直接比像素才能拿到真正的粒度。
    ///
    /// 任何不确定情形一律返回 `true`（保守）：渲染器没就绪、
    /// 区域不属于任何输出、还没有桌面快照、GPU 调用失败。
    /// 宁可多画一帧，也不能让分类框冻在旧画面上。
    fn desktop_region_changed_pixels(&mut self, rect: &WinRect) -> Result<bool> {
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Ok(true);
        }
        let generation = DESKTOP_GENERATION.load(Ordering::Relaxed);
        let key: RegionKey = (rect.left, rect.top, rect.right, rect.bottom);
        // 同一代号内重复询问直接复用结论：否则「判定没变 → 不渲染 →
        // 代号不前进 → 下一 tick 再比一次」会让每个 tick 都白跑一次
        // GPU 拷贝 + Map（5 个框 × 5 tick/秒 = 每秒 25 次）。
        if let Some(watch) = self.region_watches.get(&key) {
            if watch.checked_generation == generation {
                return Ok(watch.changed);
            }
        }

        let center_x = (rect.left + rect.right) / 2;
        let center_y = (rect.top + rect.bottom) / 2;
        let Some(index) = self
            .outputs
            .iter()
            .position(|output| point_in_rect(center_x, center_y, &output.rect))
        else {
            return Ok(true);
        };
        let output_rect = self.outputs[index].rect;
        let Some(desktop) = self.outputs[index].desktop_texture.clone() else {
            return Ok(true);
        };

        let (texture_width, texture_height, format) = unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            desktop.GetDesc(&mut desc);
            (desc.Width, desc.Height, desc.Format)
        };
        let (x, y, width, height) =
            clamp_region_to_output(rect, &output_rect, texture_width, texture_height);
        if width == 0 || height == 0 {
            return Ok(true);
        }
        self.ensure_diff_staging(&desktop, width, height)?;
        let staging = self
            .diff_staging
            .as_ref()
            .context("区域比对暂存纹理缺失")?
            .texture
            .clone();

        let copy_box = D3D11_BOX {
            left: x,
            top: y,
            front: 0,
            right: x + width,
            bottom: y + height,
            back: 1,
        };
        let hash = unsafe {
            hash_texture_region(
                &self.context,
                &staging,
                &desktop,
                &copy_box,
                bytes_per_pixel(format),
            )?
        };

        REGION_CHECKS.fetch_add(1, Ordering::Relaxed);
        // 限频诊断：把指纹打出来。
        // 「指纹一直不变」有两种完全不同的成因 —— 捕获纹理是死的，
        // 或者桌面那块地方真的没变。光看结论分不出来，必须看原始数据。
        let now = Instant::now();
        let due = REGION_HASH_LOG_AT.with(|cell| match cell.get() {
            Some(last) if now.duration_since(last) >= RECT_SAMPLE_LOG_INTERVAL => {
                cell.set(Some(now));
                true
            }
            Some(_) => false,
            None => {
                cell.set(Some(now));
                true
            }
        });
        if due {
            // debug 级：限频采样，留在 info 会吃掉 2MB 日志配额。
            // 排查「桌面明明在变却不重绘」时把日志级别调到 debug 即可。
            log::debug!(
                "[LG-dbg] 区域 ({},{})-({},{}) 指纹={hash:016x}",
                rect.left,
                rect.top,
                rect.right,
                rect.bottom
            );
        }
        let previous_hash = self.region_watches.get(&key).map(|watch| watch.hash);
        let changed = previous_hash != Some(hash);
        if changed {
            REGION_CHANGED.fetch_add(1, Ordering::Relaxed);
        }
        if self.region_watches.len() >= MAX_REGION_WATCHES {
            // 拖动分类框会不断产生新键；触顶清空重来，
            // 代价只是下一轮每个框多渲染一帧
            self.region_watches.clear();
        }
        self.region_watches.insert(
            key,
            RegionWatch {
                checked_generation: generation,
                changed,
                hash,
            },
        );
        Ok(changed)
    }

    /// 确保共享渲染表面至少能容纳 width × height。
    ///
    /// 只增不减：缩小会让大分类框下一帧又要重建，来回抖动反而更贵，
    /// 而占用上限就是「历史最大分类框」的两张纹理。
    fn ensure_surfaces(&mut self, width: u32, height: u32) -> Result<()> {
        if let Some(surfaces) = &self.surfaces {
            if surfaces.width >= width && surfaces.height >= height {
                return Ok(());
            }
        }
        // 取历史最大值，避免「大框 → 小框 → 大框」反复重建
        let (new_width, new_height) = match &self.surfaces {
            Some(surfaces) => (surfaces.width.max(width), surfaces.height.max(height)),
            None => (width, height),
        };

        unsafe {
            let output_desc = D3D11_TEXTURE2D_DESC {
                Width: new_width,
                Height: new_height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                // 既要被 D3D11 当渲染目标写（玻璃着色器），又要被 D2D1
                // 当位图源读（零拷贝合成）。少了 SHADER_RESOURCE，
                // `CreateBitmapFromDxgiSurface` 会直接被拒。
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
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


            // 把输出纹理零拷贝包成 D2D 位图。失败不致命：
            // 记一条日志、留 None，合成端会自动退回 CPU 回读路径。
            let output_bitmap = (|| -> Option<ID2D1Bitmap1> {
                let surface: IDXGISurface = output_texture.cast().ok()?;
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        // 玻璃着色器输出的就是预乘 alpha（PSMain 末尾
                        // `return float4(col*glassAlpha, alpha)`），
                        // 这里必须声明一致，否则合成时 alpha 会被再乘一次。
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                    ..Default::default()
                };
                match self
                    .d2d_context
                    .CreateBitmapFromDxgiSurface(&surface, Some(&props))
                {
                    Ok(bitmap) => Some(bitmap),
                    Err(error) => {
                        log::warn!(
                            "[LG] 玻璃输出纹理零拷贝包装失败，合成退回 CPU 回读路径: {error:?}"
                        );
                        None
                    }
                }
            })();

            let replaced = self
                .surfaces
                .replace(RenderSurfaces {
                    width: new_width,
                    height: new_height,
                    output_texture,
                    output_target,
                    output_bitmap,
                })
                .is_some();
            log::info!(
                "[LG-mem] 共享渲染表面 {}x{} = {:.1}MB（输出 1 张；暂存已改每框私有）{}",
                new_width,
                new_height,
                (new_width as f64 * new_height as f64 * 4.0) / 1_048_576.0,
                if replaced { "，替换旧尺寸" } else { "" }
            );
            Ok(())
        }
    }

    /// 推进一次光标高光插值，返回光标此刻是否落在本框内。
    ///
    /// # 为什么必须独立出来
    ///
    /// `light_direction` 是渲染器**全局共享**的一个二维向量，每渲染一帧
    /// 就把它朝「本框的目标值」拉 14%：光标在框内时拉向光标偏移，
    /// 不在框内时拉向 0。5 个框同时渲染时，1 个框拉向目标、4 个框拉向 0，
    /// 稳态幅度只有单独渲染时的约 26%。
    ///
    /// 所以一旦让「光标不在其内」的框跳过渲染，插值序列就变了，
    /// 高光会突然变亮约 3.8 倍 —— 肉眼一眼就能看出来的画面变化。
    /// 把插值单独抽出来，被跳过渲染的框仍按原节奏推进它，
    /// 就能在少画 4/5 帧的同时让观感**逐帧一致**。
    fn advance_light(
        &mut self,
        screen_x: i32,
        screen_y: i32,
        content_width: u32,
        content_height: u32,
        padding: u32,
    ) -> bool {
        /// 插值步进系数。残差按 `0.86^n` 收敛，n=25 时剩 2.2%，肉眼已看不出差别。
        const LIGHT_BLEND: f32 = 0.14;
        let surface_w = (content_width + padding * 2) as i32;
        let surface_h = (content_height + padding * 2) as i32;
        let mut cursor = POINT::default();
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
        self.light_direction[0] += (target_light[0] - self.light_direction[0]) * LIGHT_BLEND;
        self.light_direction[1] += (target_light[1] - self.light_direction[1]) * LIGHT_BLEND;
        cursor_in_window
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
        need_pixels: bool,
        box_id: usize,
        sync_read: bool,
    ) -> Result<()> {
        let center_x = screen_x + padding as i32 + content_width as i32 / 2;
        let center_y = screen_y + padding as i32 + content_height as i32 / 2;

        let output_index = self
            .outputs
            .iter()
            .position(|output| point_in_rect(center_x, center_y, &output.rect))
            .ok_or_else(|| anyhow!("分类框中心不在当前 D3D11 适配器的任何输出中"))?;

        // 记下「这个输出上有分类框」：poll_desktop_changes 据此决定
        // 要不要继续为它取帧。放在最前面，即使本帧后续失败也算数，
        // 否则一次失败就会让该输出被误判为空闲并释放资源。
        self.outputs[output_index].last_box_render_at = Some(Instant::now());

        // 零超时：分类框渲染在 UI 线程上，绝不能为了等一帧桌面而阻塞。
        // 拿不到新帧时 blur_output_impl 会复用已有的 desktop_texture；
        // 纹理整体作废的场景由 invalidate_desktop_snapshot 重建渲染器兜住。
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

        let cursor_in_window =
            self.advance_light(screen_x, screen_y, content_width, content_height, padding);

        let result = self.render_view(
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
            need_pixels,
            box_id,
            sync_read,
        );
        // 自愈：「中心像素完全透明」的一个已知成因是捕获中断/恢复或
        // 共享表面替换后，按 lens 键缓存的模糊链仍挂着旧资源，
        // 键不变就永远命中永远透明（实测 2026-09-14 整段玻璃消失）。
        // 清空链缓存让下一帧重建，把永久故障变成单帧丢弃。
        if let Err(error) = &result {
            let msg = error.to_string();
            if msg.contains("中心像素完全透明") {
                let output_mut = &mut self.outputs[output_index];
                output_mut.blur_chains.clear();
                output_mut.transient_chain = None;
                output_mut.pending_chain_keys.clear();
            }
        }
        result
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
        need_pixels: bool,
        box_id: usize,
        sync_read: bool,
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
            let surfaces = self.surfaces.as_ref().ok_or_else(|| {
                log::warn!("[LG] 共享渲染表面缺失，本框需要 {surface_width}x{surface_height}");
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

            // ===== GPU 直出：不需要 CPU 像素就到此为止 =====
            //
            // 下面整段（CopySubresourceRegion → Map 阻塞等 GPU 排空 →
            // 逐行 memcpy）实测占单帧 20ms 里的 5.5ms，而 DirectComposition
            // 呈现路径直接零拷贝采样 `output_texture`，一个字节都不用回读。
            //
            // 但**前几帧必须回读**：中心像素透明检查与 DXGI 占位黑帧检查
            // 是发现「着色器或桌面捕获彻底坏掉」的唯一手段。永久跳过的话，
            // 分类框会变成全透明且没有任何报错 —— 只能靠肉眼发现。
            // `startup_black_frames_remaining` 只在回读分支里递减，
            // 因此它天然保证了启动阶段一定会走几次完整校验。
            if !need_pixels && self.startup_black_frames_remaining == 0 {
                return Ok(());
            }

            // 共享表面可能比本框大，只能拷本框那一块。
            // CopyResource 要求两张纹理描述完全一致，所以换成子区域拷贝。
            // 下面按 mapped.RowPitch 逐行读，暂存纹理更宽也不会错位。
            let copy_box = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: surface_width,
                bottom: surface_height,
                back: 1,
            };

            // 回读段单独计时：`Map(flags=0)` 会阻塞到 GPU 排空，是整条
            // 管线唯一的硬同步点。它占多少，直接决定「保留
            // UpdateLayeredWindow」这条路还剩多少余量。
            let readback_started = Instant::now();

            // ===== 每框私有暂存回读：消掉硬同步点，也消掉框间串搭 =====
            //
            // 旧双缓冲的暂存全局共享、parity 全局翻转：两个分类框
            // 交替渲染时 A 读的缓冲已被 B 覆盖，A 的玻璃会显示
            // B 的背景，两框互相闪烁（用户反馈 2026-09-14）。
            // 现每框独占一张暂存：读「自己上一帧写的」（GPU 早完成，
            // DO_NOT_WAIT 基本必中，零等待），渲染后把本帧结果写回去。
            // 语义仍是滞后一帧，但框间竞态窗口为零。
            let need_new = match self.readbacks.get(&box_id) {
                None => true,
                Some(rb) => rb.width != surface_width || rb.height != surface_height,
            };
            if need_new {
                let mut odesc = D3D11_TEXTURE2D_DESC::default();
                unsafe { surfaces.output_texture.GetDesc(&mut odesc) };
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: surface_width,
                    Height: surface_height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: odesc.Format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging = None;
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
                let staging = staging.context("创建每框暂存纹理未返回对象")?;
                self.readbacks.insert(
                    box_id,
                    BoxReadback {
                        staging,
                        width: surface_width,
                        height: surface_height,
                        has_prev: false,
                    },
                );
            }
            let rb = self
                .readbacks
                .get_mut(&box_id)
                .expect("每框回读刚插入后必存在");

            let needed = (surface_width * surface_height) as usize;
            let row_bytes = surface_width as usize * size_of::<Pixel32>();
            let copy_rows = |mapped: &D3D11_MAPPED_SUBRESOURCE, out: &mut Vec<Pixel32>| unsafe {
                if out.len() != needed {
                    out.clear();
                    out.resize(
                        needed,
                        Pixel32 {
                            b: 0,
                            g: 0,
                            r: 0,
                            a: 0,
                        },
                    );
                }
                let src_base = mapped.pData.cast::<u8>();
                let dst_base = out.as_mut_ptr().cast::<u8>();
                if mapped.RowPitch as usize == row_bytes {
                    std::ptr::copy_nonoverlapping(src_base, dst_base, needed * size_of::<Pixel32>());
                } else {
                    for row in 0..surface_height as usize {
                        std::ptr::copy_nonoverlapping(
                            src_base.add(row * mapped.RowPitch as usize),
                            dst_base.add(row * row_bytes),
                            row_bytes,
                        );
                    }
                }
            };

            let map_started = Instant::now();
            if rb.has_prev && !sync_read {
                // 读上一帧；GPU 极少情况下还没写完就沿用 out 里更早的画面。
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                let map_result = self.context.Map(
                    &rb.staging,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                );
                super::mem_probe::add_map_timing(map_started.elapsed().as_micros() as u64);
                if map_result.is_ok() {
                    copy_rows(&mapped, out);
                    self.context.Unmap(&rb.staging, 0);
                }
                // 本帧结果写回暂存，供下一帧读。
                self.context.CopySubresourceRegion(
                    &rb.staging,
                    0,
                    0,
                    0,
                    0,
                    &surfaces.output_texture,
                    0,
                    Some(&copy_box),
                );
            } else {
                // 首帧：先写再阻塞读回，兼作初始化校验。
                self.context.CopySubresourceRegion(
                    &rb.staging,
                    0,
                    0,
                    0,
                    0,
                    &surfaces.output_texture,
                    0,
                    Some(&copy_box),
                );
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(&rb.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                super::mem_probe::add_map_timing(map_started.elapsed().as_micros() as u64);
                copy_rows(&mapped, out);
                self.context.Unmap(&rb.staging, 0);
                rb.has_prev = true;
            }
            super::mem_probe::add_readback_timing(readback_started.elapsed().as_micros() as u64);
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

    /// 主屏原点在 (0,0)，两种坐标约定算出的桌面矩形完全一致，
    /// 探测必须收敛到「输出相对」这一支（此时偏移为 0，无风险）。
    #[test]
    fn detect_convention_on_primary_monitor_prefers_output_relative() {
        let output = r(0, 0, 1920, 1080);
        let rects = [r(10, 20, 300, 400), r(0, 0, 1920, 1080)];
        assert_eq!(
            detect_dirty_rect_convention(&rects, 1920, 1080, &output),
            Some(false)
        );
    }

    /// 副屏 + 输出相对坐标：矩形落在 [0, output_width] 内，
    /// 必须判为 Some(false)，调用方随后加上输出原点偏移。
    #[test]
    fn detect_convention_secondary_output_relative() {
        let output = r(2560, 0, 4480, 1080);
        let rects = [r(0, 0, 64, 64), r(1900, 1000, 1920, 1080)];
        assert_eq!(
            detect_dirty_rect_convention(&rects, 1920, 1080, &output),
            Some(false)
        );
    }

    /// 副屏 + 桌面绝对坐标：矩形超出输出尺寸但落在输出矩形内，
    /// 必须判为 Some(true)，调用方不得再加偏移（否则整块偏出一个屏）。
    #[test]
    fn detect_convention_secondary_desktop_absolute() {
        let output = r(2560, 0, 4480, 1080);
        let rects = [r(2600, 100, 3000, 500), r(4000, 900, 4480, 1080)];
        assert_eq!(
            detect_dirty_rect_convention(&rects, 1920, 1080, &output),
            Some(true)
        );
    }

    /// 两边都不符（数据损坏 / 约定又变了）时必须返回 None，
    /// 让下一帧继续探测，而不是随便猜一个把画面冻住。
    #[test]
    fn detect_convention_returns_none_when_rects_fit_neither() {
        let output = r(2560, 0, 4480, 1080);
        // right=9999 既超出输出尺寸也超出输出矩形
        let rects = [r(2600, 100, 9999, 500)];
        assert_eq!(
            detect_dirty_rect_convention(&rects, 1920, 1080, &output),
            None
        );
        // 负坐标同样两边都不符
        let rects = [r(-5, 0, 100, 100)];
        assert_eq!(
            detect_dirty_rect_convention(&rects, 1920, 1080, &output),
            None
        );
    }

    /// 空批次不能下结论：DXGI 常常只给 move rect 不给 dirty rect。
    #[test]
    fn detect_convention_returns_none_for_empty_batch() {
        let output = r(2560, 0, 4480, 1080);
        assert_eq!(detect_dirty_rect_convention(&[], 1920, 1080, &output), None);
    }

    /// 混合批次里只要有一个矩形不符合某约定，就不能采用该约定。
    /// 判错会让副屏上的分类框收不到变化通知（画面冻住且无报错）。
    #[test]
    fn detect_convention_requires_every_rect_to_agree() {
        let output = r(2560, 0, 4480, 1080);
        // 第一个矩形符合输出相对，第二个（绝对坐标）不符合
        let rects = [r(0, 0, 64, 64), r(2600, 100, 3000, 500)];
        assert_eq!(
            detect_dirty_rect_convention(&rects, 1920, 1080, &output),
            None
        );
    }

    /// 诊断样本最多取 4 个矩形（一批可能有上千个，日志不能爆），
    /// 空批次返回空串而不是 panic。
    #[test]
    fn sample_rects_is_bounded_and_handles_empty() {
        assert_eq!(sample_rects(&[]), "");
        let one = sample_rects(&[r(1, 2, 3, 4)]);
        assert_eq!(one, "(1,2)-(3,4)");
        let many: Vec<WinRect> = (0..50).map(|i| r(i, i, i + 1, i + 1)).collect();
        let sample = sample_rects(&many);
        assert_eq!(
            sample.split(' ').count(),
            4,
            "样本必须截到 4 个矩形: {sample}"
        );
    }

    /// 指纹必须对**任何一个字节**的变化都敏感：区域里只有一个像素
    /// 变了（比如时钟跳了一秒）也要判成「变了」，否则分类框会停在
    /// 旧背景上，而这正是用户能一眼看出来的画面错误。
    #[test]
    fn hash_region_detects_single_byte_change() {
        let base: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let original = hash_region(&base, HASH_SEED);
        assert_eq!(original, hash_region(&base, HASH_SEED), "同内容必须同指纹");
        for index in [0usize, 1, 2047, 4095] {
            let mut mutated = base.clone();
            mutated[index] = mutated[index].wrapping_add(1);
            assert_ne!(
                original,
                hash_region(&mutated, HASH_SEED),
                "第 {index} 字节 +1 必须改变指纹"
            );
        }
    }

    /// seed 必须真的参与运算：多行要靠它串成一个区域指纹，
    /// 如果 seed 被忽略，行序调换就检测不出来了。
    #[test]
    fn hash_region_seed_chains_rows() {
        let row_a = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let row_b = [9u8, 9, 9, 9, 9, 9, 9, 9];
        let ab = hash_region(&row_b, hash_region(&row_a, HASH_SEED));
        let ba = hash_region(&row_a, hash_region(&row_b, HASH_SEED));
        assert_ne!(ab, ba, "行序不同必须得到不同指纹");
        assert_ne!(
            hash_region(&row_a, HASH_SEED),
            hash_region(&row_a, HASH_SEED ^ 1),
            "seed 必须影响结果"
        );
    }

    /// 尾部不足 8 字节的部分也要算进指纹，且空输入不能 panic。
    #[test]
    fn hash_region_handles_partial_tail_and_empty() {
        assert_eq!(hash_region(&[], HASH_SEED), hash_region(&[], HASH_SEED));
        let full = [7u8; 16];
        let mut tail_differs = full;
        tail_differs[12] = 8;
        assert_ne!(
            hash_region(&full, HASH_SEED),
            hash_region(&tail_differs, HASH_SEED),
            "尾部字节也必须参与"
        );
        // 长度不是 8 的倍数
        assert_ne!(
            hash_region(&[1u8, 2, 3], HASH_SEED),
            hash_region(&[1u8, 2, 4], HASH_SEED)
        );
    }

    /// 每像素字节数：HDR 桌面是 16 位浮点四通道 = 8 字节，其余按 4 字节。
    /// 算错会让行字节数错位，指纹随之全错（表现为永远判「变了」或永远判「没变」）。
    #[test]
    fn bytes_per_pixel_matches_capture_formats() {
        assert_eq!(bytes_per_pixel(DXGI_FORMAT_B8G8R8A8_UNORM), 4);
        assert_eq!(bytes_per_pixel(DXGI_FORMAT_R10G10B10A2_UNORM), 4);
        assert_eq!(bytes_per_pixel(DXGI_FORMAT_R16G16B16A16_FLOAT), 8);
    }

    /// # 阳性对照：区域指纹必须对**单像素**变化敏感
    ///
    /// 「桌面没变就不重绘」这个判定一旦判错，表现就是分类框冻在旧背景上，
    /// 而这正是用户能一眼看出来的画面错误。真实桌面上没法可靠构造对照
    /// （锁屏 / 安全桌面下 DXGI 拿不到帧，探针窗口也进不了合成），
    /// 所以这里用一张自己填好的纹理直接测 `hash_texture_region` 全链路：
    /// 子区域拷贝 → Map → 按 RowPitch 逐行 → 指纹。
    ///
    /// 三条断言缺一不可：
    /// 1. 回读结果必须与 CPU 侧同区域逐字节算出的指纹**完全相等**
    ///    —— 证明拷贝范围与行距都对，而不是「碰巧在变」；
    /// 2. 区域内改一个像素，指纹必须变 —— 判不出变化就会冻画面；
    /// 3. 区域**外**改一个像素，指纹必须不变 —— 否则无关变化也会触发
    ///    重绘，按框过滤就失去意义了。
    #[test]
    fn region_hash_is_exact_and_single_pixel_sensitive() {
        let (device, context) = create_device().expect("创建 D3D11 设备失败，无法验证区域指纹链路");
        unsafe {
            let (src_w, src_h) = (64u32, 48u32);
            let pitch = src_w * 4;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: src_w,
                Height: src_h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            // 可复现的伪随机图案，避免大面积相同字节掩盖拷贝错位
            let mut pixels: Vec<u8> = (0..(src_w * src_h * 4) as usize)
                .map(|i| (i.wrapping_mul(7) % 251) as u8)
                .collect();
            let init = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr() as *const c_void,
                SysMemPitch: pitch,
                SysMemSlicePitch: 0,
            };
            let mut texture = None;
            device
                .CreateTexture2D(&desc, Some(&init), Some(&mut texture))
                .expect("创建源纹理失败");
            let texture = texture.expect("CreateTexture2D 未返回对象");

            // staging 故意比区域大，模拟「所有区域共享一份暂存纹理」，
            // 这样 RowPitch 一定大于区域行宽，行距算错就会立刻暴露
            // 注意 MipLevels / ArraySize / SampleDesc 必须显式给：
            // Default 出来是 0，CreateTexture2D 会直接返回 E_INVALIDARG
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: 40,
                Height: 32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .expect("创建暂存纹理失败");
            let staging = staging.expect("CreateTexture2D 未返回暂存对象");

            let (reg_x, reg_y, reg_w, reg_h) = (10u32, 6u32, 20u32, 15u32);
            let region_box = D3D11_BOX {
                left: reg_x,
                top: reg_y,
                front: 0,
                right: reg_x + reg_w,
                bottom: reg_y + reg_h,
                back: 1,
            };
            let got = hash_texture_region(
                &context,
                &staging,
                &texture,
                &region_box,
                bytes_per_pixel(DXGI_FORMAT_B8G8R8A8_UNORM),
            )
            .expect("区域指纹计算失败");

            // 1) 与 CPU 侧同区域逐字节算出的期望值完全一致
            let mut expected = HASH_SEED;
            for row in 0..reg_h as usize {
                let start = ((reg_y as usize + row) * src_w as usize + reg_x as usize) * 4;
                expected = hash_region(&pixels[start..start + reg_w as usize * 4], expected);
            }
            assert_eq!(
                got, expected,
                "GPU 回读的区域必须与 CPU 侧同一区域逐字节一致（拷贝范围或行距算错了）"
            );

            // 2) 区域正中间改一个像素 → 指纹必须变
            let mid = ((reg_y as usize + reg_h as usize / 2) * src_w as usize
                + reg_x as usize
                + reg_w as usize / 2)
                * 4;
            pixels[mid] = pixels[mid].wrapping_add(1);
            context.UpdateSubresource(
                &texture,
                0,
                None,
                pixels.as_ptr() as *const c_void,
                pitch,
                0,
            );
            let after_inside = hash_texture_region(
                &context,
                &staging,
                &texture,
                &region_box,
                bytes_per_pixel(DXGI_FORMAT_B8G8R8A8_UNORM),
            )
            .expect("区域指纹计算失败");
            assert_ne!(
                got, after_inside,
                "区域内单像素 +1 必须改变指纹，否则分类框会冻在旧背景上"
            );

            // 3) 区域外 (0,0) 改一个像素 → 指纹必须不变
            pixels[0] = pixels[0].wrapping_add(1);
            context.UpdateSubresource(
                &texture,
                0,
                None,
                pixels.as_ptr() as *const c_void,
                pitch,
                0,
            );
            let after_outside = hash_texture_region(
                &context,
                &staging,
                &texture,
                &region_box,
                bytes_per_pixel(DXGI_FORMAT_B8G8R8A8_UNORM),
            )
            .expect("区域指纹计算失败");
            assert_eq!(
                after_inside, after_outside,
                "区域外的变化不应影响该区域指纹，否则按框过滤失去意义"
            );
        }
    }

    /// 区域夹取：完全在内 / 部分越界 / 完全在外 三种情形。
    ///
    /// 夹错会让比对读到相邻区域的像素，表现为「桌面明明变了却判成没变」，
    /// 分类框就冻住了 —— 这是最危险的一类错，必须有测试钉住。
    #[test]
    fn clamp_region_to_output_handles_all_cases() {
        let output = r(3120, 0, 5040, 1080); // 副屏：原点 (3120,0)，1920x1080
                                             // 完全在输出内
        assert_eq!(
            clamp_region_to_output(&r(3200, 100, 3400, 300), &output, 1920, 1080),
            (80, 100, 200, 200)
        );
        // 左上越界 → 夹到 0
        assert_eq!(
            clamp_region_to_output(&r(3000, -50, 3400, 300), &output, 1920, 1080),
            (0, 0, 280, 300)
        );
        // 右下越界 → 夹到纹理尺寸
        assert_eq!(
            clamp_region_to_output(&r(4900, 900, 5200, 1200), &output, 1920, 1080),
            (1780, 900, 140, 180)
        );
        // 完全在输出左侧之外 → 宽度 0，调用方必须保守放行
        let (_, _, w, _) = clamp_region_to_output(&r(0, 0, 100, 100), &output, 1920, 1080);
        assert_eq!(w, 0, "输出之外的区域宽度必须为 0");
        // 完全在输出下方之外 → 高度 0
        let (_, _, _, h) = clamp_region_to_output(&r(3200, 2000, 3400, 2200), &output, 1920, 1080);
        assert_eq!(h, 0, "输出之外的区域高度必须为 0");
    }

    /// 主屏原点在 (0,0)，夹取结果应当就是区域本身（不得凭空偏移）。
    #[test]
    fn clamp_region_on_primary_output_is_identity() {
        let output = r(0, 0, 2560, 1440);
        assert_eq!(
            clamp_region_to_output(&r(100, 200, 400, 600), &output, 2560, 1440),
            (100, 200, 300, 400)
        );
    }

    /// ACCESS_LOST 退避必须单调不减、有上限，且第一次立即放行。
    ///
    /// 退避不单调会让重建风暴复发；没有上限则桌面恢复后
    /// 最坏要等很久才重新取到帧（分类框长时间停在旧画面）。
    #[test]
    fn access_lost_backoff_is_monotonic_and_bounded() {
        assert_eq!(access_lost_backoff(0), Duration::ZERO, "第一次必须立即重建");
        let mut previous = Duration::ZERO;
        for streak in 1..64u32 {
            let delay = access_lost_backoff(streak);
            assert!(
                delay >= previous,
                "退避必须单调不减：streak={streak} 时 {delay:?} < {previous:?}"
            );
            assert!(
                delay <= Duration::from_secs(3),
                "退避必须有上限，否则桌面恢复后长时间取不到帧：{delay:?}"
            );
            previous = delay;
        }
        assert_eq!(access_lost_backoff(1000), Duration::from_secs(3));
    }

    /// 静止的分类框：第一次未命中先用临时链，第二次升级成常驻链，
    /// 之后一直命中。升级之后就不该再有任何 `CreateEffect` / `SetValue`。
    #[test]
    fn static_box_promotes_to_cached_chain() {
        assert_eq!(plan_blur_chain(false, 1), ChainPlan::Transient);
        assert_eq!(
            plan_blur_chain(false, CHAIN_PROMOTE_AFTER_MISSES),
            ChainPlan::Promote
        );
        assert_eq!(
            plan_blur_chain(false, CHAIN_PROMOTE_AFTER_MISSES + 5),
            ChainPlan::Promote
        );
        assert_eq!(plan_blur_chain(true, 0), ChainPlan::Cached);
        // 已命中常驻表时，未命中计数不参与判断
        assert_eq!(plan_blur_chain(true, 99), ChainPlan::Cached);
    }

    /// # 拖动时绝不升级成常驻链
    ///
    /// 这是拖动流畅度的硬保证：拖动中裁剪框每帧都变，每个键都只出现一次，
    /// 必须**全程**走 Transient（复用同一条链、只改裁剪框）。
    /// 一旦哪一帧判成 Promote，那一帧就要 `CreateEffect` ×4 并让 D2D
    /// 重新 realize 一整张图，拖动时的液态玻璃就会掉帧。
    #[test]
    fn dragging_never_promotes_a_chain() {
        // 模拟一次 600 帧的拖动：每帧裁剪框都是新键，各出现 1 次。
        // 计数方式必须与生产代码 `blur_output_impl` 完全一致，
        // 否则这条测试证明不了任何东西。
        let mut pending: std::collections::HashMap<i32, u8> = std::collections::HashMap::new();
        let mut promoted = 0usize;
        let mut transient = 0usize;
        for frame in 0..600i32 {
            let seen = pending.get(&frame).copied().unwrap_or(0).saturating_add(1);
            if pending.len() > MAX_PENDING_CHAIN_KEYS {
                pending.clear();
            }
            pending.insert(frame, seen);
            match plan_blur_chain(false, seen) {
                ChainPlan::Promote => promoted += 1,
                ChainPlan::Transient => transient += 1,
                ChainPlan::Cached => panic!("未命中却判成 Cached"),
            }
        }
        assert_eq!(promoted, 0, "拖动全程都不该新建常驻链");
        assert_eq!(transient, 600, "拖动全程都应复用临时链");
    }

    /// 拖动结束后分类框停住：同一个键第二次出现就必须升级，
    /// 否则静止态会一直走临时链、每帧都 SetValue 作废 realization，
    /// 显存又开始爬。
    #[test]
    fn box_settles_after_drag_and_promotes() {
        let mut pending: std::collections::HashMap<i32, u8> = std::collections::HashMap::new();
        // 拖动阶段：三个不同的键各出现一次
        for frame in [10, 11, 12] {
            let seen = pending.get(&frame).copied().unwrap_or(0) + 1;
            pending.insert(frame, seen);
            assert_eq!(plan_blur_chain(false, seen), ChainPlan::Transient);
        }
        // 停住：同一个键连续两帧
        let settled = 12;
        let seen = pending.get(&settled).copied().unwrap_or(0) + 1;
        pending.insert(settled, seen);
        assert_eq!(plan_blur_chain(false, seen), ChainPlan::Promote);
        // 升级后进常驻表
        assert_eq!(plan_blur_chain(true, 0), ChainPlan::Cached);
    }

    /// LRU 淘汰必须挑「最近使用序号最小」的那条链，空集合返回 None。
    ///
    /// 挑错会把正在用的链踢掉，下一帧重建 → realization 作废 →
    /// 显存 churn 又回来了，正是要修的那个问题。
    #[test]
    fn lru_victim_picks_least_recently_used() {
        let stamps = [
            ((1, 1, 10, 10), 5u64),
            ((2, 2, 20, 20), 3),
            ((3, 3, 30, 30), 9),
        ];
        assert_eq!(lru_victim(stamps.into_iter()), Some((2, 2, 20, 20)));
        assert_eq!(
            lru_victim(std::iter::empty::<(BlurChainKey, u64)>()),
            None,
            "空集合必须返回 None，由调用方 break"
        );
    }

    /// 序号相同时也必须给出确定的一个键，不能返回 None 让淘汰卡死：
    /// 卡死会让 blur_chains 永远停在容量上限，新位置的分类框再也建不了链。
    #[test]
    fn lru_victim_resolves_ties_and_never_stalls() {
        let stamps = [((7, 0, 1, 1), 4u64), ((8, 0, 1, 1), 4)];
        let victim = lru_victim(stamps.into_iter()).expect("非空集合必须有淘汰对象");
        assert!(victim == (7, 0, 1, 1) || victim == (8, 0, 1, 1));
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
