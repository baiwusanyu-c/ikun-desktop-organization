# 优化分类框边缘拖拽调整大小

## Goal

重新分布分类框（格子窗口）的边缘拖拽 resize handle：body 顶部区域不再支持 resize，改由 header 区域接管顶部的 resize 控制。

## What I already know

* 格子窗口是独立的 WS_POPUP 分层窗口（`src/desktop/grid_window.rs`）
* resize 通过 WM_NCHITTEST 返回 HT* 常量实现，Windows 原生处理缩放
* 窗口结构：header（标题栏，高度 `title_h`，约 40px DPI 缩放）+ body（SysListView32 子窗口）
* **当前 body 区域的 resize handle**（代码行 866-894）：
  - ✅ 左下角 (HTBOTTOMLEFT)
  - ✅ 右下角 (HTBOTTOMRIGHT)
  - ✅ 左上角 (HTTOPLEFT) — **需移除**
  - ✅ 右上角 (HTTOPRIGHT) — **需移除**
  - ✅ 左边缘 (HTLEFT)
  - ✅ 右边缘 (HTRIGHT)
  - ✅ 底边缘 (HTBOTTOM)
  - ✅ 顶部边缘区域 (on_top_body) — **需移除**
* **当前 header 区域**（代码行 896-899）：
  - 整个区域返回 HTCAPTION（拖拽移动），无 resize handle

## Requirements

* R1: body 区域移除 HTTOPLEFT、HTTOPRIGHT resize handle
* R2: body 区域移除 on_top_body 边缘检测（body 顶部不再有 resize 区域）
* R3: header 区域添加 HTTOPLEFT（左上角）
* R4: header 区域添加 HTTOPRIGHT（右上角）
* R5: header 区域添加 HTTOP（顶部边缘）
* R6: header 非边缘区域保持 HTCAPTION（拖拽移动）
* R7: body 的左/右/底边缘和底部两角保持不变

## Acceptance Criteria

* [x] 鼠标在 body 左上角区域不显示 resize 光标
* [x] 鼠标在 body 右上角区域不显示 resize 光标
* [x] 鼠标在 body 顶部边缘不显示 resize 光标
* [x] 鼠标在 header 左上角显示 SIZENWSE 光标并可拖拽调整大小
* [x] 鼠标在 header 右上角显示 SIZENESW 光标并可拖拽调整大小
* [x] 鼠标在 header 顶部边缘显示 SIZENS 光标并可拖拽调整大小
* [x] header 中间区域仍可拖拽移动窗口（HTCAPTION）
* [x] body 底部三个 resize handle（左下、右下、底边）仍正常工作
* [x] body 左/右边缘 resize 仍正常工作

## Definition of Done

* 代码修改后 `cargo check` 通过
* `cargo clippy` 无新增警告
* 手动测试验证所有 resize handle 位置正确

## Decision (ADR-lite)

**Context**: 用户要求优化 resize handle 分布，使 header 接管顶部 resize，body 移除顶部 resize
**Decision**: header 仅处理顶部三角/边缘（HTTOPLEFT, HTTOPRIGHT, HTTOP），不处理左/右边缘。body 保留左/右/底部 resize。
**Consequences**: header 的左/右侧和底部角落区域保持 HTCAPTION 拖拽行为。用户从 header 区域只能通过顶部调整大小，移动操作占据更大区域。

## Out of Scope

* 不修改 resize 灵敏度/edge 检测范围
* 不修改 WM_SETCURSOR 光标映射逻辑
* 不修改 body 底部和两侧的 resize handle
* 不修改 header 绘制、ListView 嵌入等其他功能

## Technical Notes

* 修改文件：`src/desktop/grid_window.rs`
* 修改位置：`grid_wnd_proc` 函数 → `WM_NCHITTEST` 分支（行 844-901）
* 当前 edge 检测阈值：`scale_for_dpi(30, dpi)` 约 30px（DPI 缩放）
* 当前 HTCAPTION 覆盖整个 header 区域（dy < title_h），需缩小为排除顶部边缘后的区域

## Hit Test 区域分布图（目标状态）

```
窗口布局（从上方俯视）：

  ┌─HTTOPLEFT──HTTOP────────HTTOPRIGHT─┐  ← 窗口顶边（header 顶部边缘）
  │                                      │
  │        HTCAPTION (拖拽移动)          │  ← header 区域（非边缘 = 拖拽）
  │                                      │
  ├──────────────────────────────────────┤  ← title_h 分界线
  │                                      │
  HTLEFT       (无 resize)         HTRIGHT  ← body 区域
  │                                      │
  │                                      │
  └─HTBOTTOMLEFT─HTBOTTOM──HTBOTTOMRIGHT┘  ← 窗口底边
```
