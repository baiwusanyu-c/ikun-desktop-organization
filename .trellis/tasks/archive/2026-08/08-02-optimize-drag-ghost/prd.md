# 优化桌面拖拽残影图标在分类框中的显示

## 目标
当从桌面拖拽图标到分类框时，保持残影图标（ghost image）的显示，而不是变成虚线小方框（OLE 默认拖拽光标）。

## 问题根因
当从桌面拖拽时：
1. Windows Explorer 作为拖拽源，使用 IDragSourceHelper 创建残影图标
2. 当光标进入分类框（IDropTarget）时，DragEnter 返回 DROPEFFECT_COPY
3. Windows 调用拖拽源的 GiveFeedback，返回 DRAGDROP_S_USEDEFAULTCURSORS
4. Windows 使用默认 OLE 拖拽光标（虚线方框）替换残影图标

## 解决方案
在 IDropTarget 回调中集成 IDropTargetHelper COM 接口，让 Windows Shell 继续管理残影图标的显示。

### 修改文件
- `src/platform/windows/ffi_types.rs`: 添加 IID_IDROPTARGETHELPER 常量
- `src/platform/windows/drop_target.rs`: 集成 IDropTargetHelper

### 关键实现细节
1. IDropTargetHelperVtbl 的 drag_enter/drag_over/drop 方法使用 `*mut POINT` 指针（不是 i64 打包值）
2. 调用时需从 i64 解包为 POINT 结构体再传指针
3. 正确的 vtable 双重指针解引用：`*(helper as *const *const IDropTargetHelperVtbl)`
4. 每个 BoxDropTarget 实例独立管理 helper 生命周期

## 历史踩坑记录

### 之前所有尝试失败的根本原因（commit 7029834）
```rust
// 错误写法：vtable 解引用少了一层指针
let vtbl = &*(helper as *const IDropTargetHelperVtbl);
// COM 对象内存布局: [vtable_ptr(8B)][refcount(4B)]...
// 把对象内存直接当 vtable 结构体 → drag_enter 字段读到的是 add_ref 函数指针
// 调用了错误的 COM 方法 → OLE 拖拽状态被破坏 → drop 永远不会触发

// 正确写法：先读出 vtable_ptr，再通过 vtable_ptr 访问方法表
let vtbl = *(helper as *const *const IDropTargetHelperVtbl);
```

## 验收结果
- [x] 从桌面拖拽图标到分类框时，残影图标保持显示
- [x] 残影图标在分类框内平滑跟随鼠标
- [x] 拖拽释放后图标正确添加到分类框
- [x] 框间拖拽功能不受影响
