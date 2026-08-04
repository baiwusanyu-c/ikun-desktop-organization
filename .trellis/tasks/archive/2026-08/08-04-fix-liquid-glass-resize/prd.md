# fix: 分类框边缘拖拽调整大小时液态玻璃效果消失

## 目标
修复分类框在边缘拖拽调整大小时，液态玻璃效果偶尔消失并退化为半透明背景的 bug。

## 已知信息
- 液态玻璃效果通过 DXGI Desktop Duplication API 捕获桌面画面，经 GPU 管线处理后渲染
- 渲染器使用 thread_local GlassState 状态机：Uninitialized -> Ready -> Failed（3 秒重试延迟）
- 边缘拖拽时 WM_SIZE 频繁触发，每次都调用 render_layered_window
- 当 AcquireNextFrame 返回 DXGI_ERROR_ACCESS_LOST 时，当前实现直接返回 Err，导致整个渲染器进入 Failed 状态
- Failed 状态持续 3 秒（RETRY_DELAY），期间所有渲染请求返回 None，退化为半透明背景

## 根因分析
1. DXGI_ERROR_ACCESS_LOST 处理不当：正确做法是重建 IDXGIOutputDuplication，而非将整个渲染器标记为失败
2. 无重建机制：当前代码没有重建 duplication 的逻辑
3. 过度惩罚：单个 output 的 access lost 不应该影响整个渲染器的状态

## 需求
- 当 DXGI_ERROR_ACCESS_LOST 发生时，自动重建受影响的 IDXGIOutputDuplication
- 重建失败时才降级为 Failed 状态
- 重建成功时继续正常渲染，不中断液态玻璃效果

## 验收标准
- [ ] 边缘拖拽调整大小时液态玻璃效果不消失
- [ ] DXGI_ERROR_ACCESS_LOST 后能自动恢复
- [ ] 编译通过，现有测试通过
