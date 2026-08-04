# bugfix: 分类框拖拽到屏幕顶部被弹性回弹

## 目标
修复分类框（grid_window）拖拽到屏幕顶部时窗口被弹回一段距离的问题，使用户可以正常将分类框拖拽到屏幕顶部任意位置。

## 已知信息
* 分类框是独立的 WS_POPUP | WS_THICKFRAME | WS_EX_LAYERED | WS_EX_TOOLWINDOW 窗口
* 窗口标题栏拖拽通过 WM_NCHITTEST 返回 HTCAPTION 实现，由 DefWindowProcW 处理标准移动
* 窗口四周有 128 逻辑像素的阴影留白 (LIQUID_GLASS_SHADOW_PADDING = 128)
* WM_WINDOWPOSCHANGING 仅干预 Z-order，不修改位置坐标
* WM_MOVING 处理对齐检测（alignment_guide），不修改 proposed_rect
* WM_GETMINMAXINFO 设置 min=200+padding, max=3000+padding，不会限制顶部移动
* 渲染使用 UpdateLayeredWindow，提交时 pptDst = null，不修改位置
* WM_ENTERSIZEMOVE/WM_EXITSIZEMOVE 用于启动/停止对齐参考线定时器

## 临时假设
* 可能与液态玻璃阴影留白（shadow_padding=128px）有关，窗口外部矩形比可见内容大 128px*2
* 可能与 Windows Aero Snap 行为冲突（拖到顶部触发最大化但分层窗口不支持）
* 可能与显示器工作区边界检测有关
* 可能渲染函数在特定位置触发了窗口位置重置

## 待解决问题
* 确认弹性回弹的具体表现：是窗口无法到达屏幕顶部（有间距），还是到达后被弹回？
* 确认回弹是瞬间发生还是在松手后发生？

## 需求（演进中）
* 分类框可以自由拖拽到屏幕任意位置，包括屏幕最顶部
* 拖拽松手后窗口停留在用户放置的位置，不回弹
* 不影响现有的对齐参考线、Z-order 管理等功能

## 验收标准（演进中）
- [ ] 分类框可以拖拽到屏幕最顶部，可见内容贴紧屏幕顶部
- [ ] 松手后窗口位置不发生变化
- [ ] 对齐参考线功能正常
- [ ] Z-order 管理正常（分类框不高于其他应用窗口）

## 完成定义（团队质量门槛）
* Lint / typecheck / CI 通过
* 手动测试拖拽到各屏幕边缘无异常

## 明确排除范围
* 不修改分类框的渲染逻辑
* 不修改对齐参考线功能
* 不修改 Z-order 管理逻辑

## 技术笔记
* 窗口创建: src/platform/windows/grid_window.rs create_grid_window()
* 窗口消息处理: src/platform/windows/wnd_proc.rs grid_wnd_proc()
* D2D 渲染: src/platform/windows/d2d_render.rs render_layered_window()
* 对齐检测: src/platform/windows/alignment_guide/mod.rs
* 阴影留白: src/platform/windows/grid_window.rs LIQUID_GLASS_SHADOW_PADDING=128
* Z-order 管理: src/platform/windows/zorder.rs
