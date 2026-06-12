# GDI 迁移到 DirectWrite/Direct2D 渲染管线

## Goal

将 `src/desktop/grid_window.rs` 中的 GDI 渲染管线替换为 Direct2D + DirectWrite，实现：
1. **文字质量提升**：DirectWrite 原生 per-pixel alpha，消除透明背景上的抗锯齿锯齿
2. **图标缩放质量提升**：Direct2D 线性插值缩放替代 DrawIconEx 的低质量缩放
3. **GPU 加速**：渲染性能提升，resize 更流畅
4. **DPI 适配**：DirectWrite 使用 DIP 单位，自动适配 DPI

## Requirements

### R1: Cargo.toml 添加 Direct2D/DirectWrite feature flags
- 添加 `Win32_Graphics_Direct2D`, `Win32_Graphics_Direct2D_Common`
- 添加 `Win32_Graphics_DirectWrite`
- 添加 `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`
- 添加 `Win32_Graphics_Wic`（WIC bitmap 支持）

### R2: 文字渲染迁移（DirectWrite）
- 用 `IDWriteFactory` + `IDWriteTextFormat` + `IDWriteTextLayout` 替代 `CreateFontW` + `DrawTextW`
- 删除 `draw_text_alpha` 函数（~130 行软件 alpha 合成 hack）
- 保留阴影描边效果（通过绘制多次 TextLayout 实现）
- 文字颜色和透明度通过 `ID2D1SolidColorBrush` 控制

### R3: 图标渲染迁移（WIC + Direct2D）
- 替换 `file_icon.rs` 中的图标提取：使用 WIC 从 exe 提取 IWICBitmap（替代 HICON）
- 保留 .lnk 解析逻辑（IShellLink + IPersistFile）
- 图标渲染使用 `ID2D1RenderTarget::DrawBitmap` + 线性插值
- 图标缓存到窗口级 GridWindowData 中（避免每帧重新提取）

### R4: 背景/形状渲染迁移（Direct2D）
- 用 `ID2D1RoundedRectangleGeometry` + `FillGeometry` 替代 `fill_rounded_rect_alpha`
- 用 `FillRectangle` + `ID2D1SolidColorBrush` 替代 `fill_rect_alpha`
- 半透明背景通过 brush alpha 值控制

### R5: 渲染管线改造
- 初始化：窗口创建时初始化 D2D/DW 设备，存储在 GridWindowData 中
- Render target：使用 `ID2D1DCRenderTarget` 绑定到 DIB（兼容 UpdateLayeredWindow）
- resize 时重建 render target（DIB 尺寸变化）
- 清理：窗口销毁时释放 D2D/DW 资源
- 保持 `UpdateLayeredWindow` 作为最终提交方式

### R6: 窗口级设备缓存
- `GridWindowData` 新增字段：
  - `d2d_render_target: Option<ID2D1DCRenderTarget>`
  - `dw_factory: Option<IDWriteFactory>`
  - `d2d_factory: Option<ID2D1Factory>`
- resize 时重建 render target（绑定到新 DIB）
- factory 对象窗口生命周期内复用

### R7: 保持现有功能不变
- 窗口创建、消息处理、hit test、拖拽/缩放
- 双击打开文件、右键菜单
- DPI 自适应（WM_DPICHANGED）
- 选中项高亮
- 图标释放（WM_DESTROY）

## Acceptance Criteria

- [ ] `cargo check` 编译通过
- [ ] `cargo clippy` 无警告
- [ ] `cargo fmt` 格式化通过
- [ ] 格子窗口能正常创建和显示
- [ ] 标题栏文字清晰、无锯齿（透明背景上）
- [ ] 图标标签文字清晰、支持 DPI 缩放
- [ ] 图标缩放质量高（线性插值，非最近邻）
- [ ] 圆角背景正确渲染
- [ ] 窗口拖拽、缩放正常
- [ ] 双击打开文件正常
- [ ] 右键菜单正常
- [ ] DPI 变化时布局自适应
- [ ] 选中项高亮正常
- [ ] 内存无泄漏（D2D/DW 资源正确释放）

## Technical Notes

### 迁移策略
- 一次性迁移（非分步）
- 全部替换 grid_window.rs 中的渲染代码
- file_icon.rs 改为 WIC 管线

### API 对应关系

| GDI（当前） | Direct2D/DirectWrite（目标） | 作用 |
|------------|---------------------------|------|
| `CreateFontW` + `DrawTextW` | `IDWriteFactory` + `IDWriteTextFormat` + `DrawTextLayout` | 文字渲染 |
| `SetBkMode(TRANSPARENT)` | `ID2D1RenderTarget` 天然透明背景 | 透明背景 |
| `SetTextColor` | `ID2D1SolidColorBrush` | 文字颜色 |
| `DrawIconEx` | `IWICBitmap` + `ID2D1Bitmap` + `DrawBitmap` | 图标渲染 |
| `fill_rounded_rect_alpha` | `ID2D1RoundedRectangle` + `FillRoundedRectangle` | 圆角矩形 |
| `fill_rect_alpha` | `ID2D1SolidColorBrush` + `FillRectangle` | 矩形填充 |
| `CreateDIBSection` + `UpdateLayeredWindow` | `ID2D1DCRenderTarget` 渲染到 DIB + `UpdateLayeredWindow` | 分层窗口 |
| `SetStretchBltMode(HALFTONE)` | `D2D1_BITMAP_INTERPOLATION_MODE_LINEAR` | 高质量缩放 |

### 关键约束
- `UpdateLayeredWindow` 仍需 DIB 作为后端 → 使用 `ID2D1DCRenderTarget`（绑定 HDC + DIB）
- D2D COM 对象需要正确的引用计数管理
- 所有 D2D/DW 初始化在 `#[cfg(windows)]` 下
- 保持 `extern "system"` FFI 声明用于非 D2D/DW 的 Win32 API
