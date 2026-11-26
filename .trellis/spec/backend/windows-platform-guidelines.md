# Windows Platform Quality Guidelines

> Windows-specific coding standards and common pitfalls

---

## Layered Windows (WS_EX_LAYERED)

### Alpha Behavior

WS_EX_LAYERED windows have **pixel-level alpha testing** that happens before WM_NCHITTEST:

- **Alpha = 0**: Pixel is transparent, mouse events pass through, WM_MOUSELEAVE fires repeatedly
- **Alpha > 0**: Pixel is opaque, receives all mouse events normally

### Common Pitfalls

#### ❌ Pitfall 1: Relying on WM_NCHITTEST for Click-Through

`
ust
// WRONG: WM_NCHITTEST is bypassed for alpha=0 pixels
WM_NCHITTEST => {
    if is_blank_area {
        HTTRANSPARENT  // System already bypassed this!
    } else {
        HTCLIENT
    }
}
`

#### ❌ Pitfall 2: Using WM_MOUSELEAVE for Hover Management

`
ust
// WRONG: WM_MOUSELEAVE fires repeatedly on alpha=0 pixels
WM_MOUSELEAVE => {
    data.hovered_item = -1;  // Causes flickering!
    render();
}
`

#### ❌ Pitfall 3: Event Forwarding via PostMessage

`
ust
// WRONG: Fighting against platform behavior
if is_blank_area {
    let desktop = FindWindowW(...);
    PostMessageW(desktop, WM_LBUTTONDOWN, ...);  // Complex and unreliable
}
`

### Correct Patterns

#### ✅ Pattern 1: State-Driven Alpha

`
ust
// CORRECT: Dynamic alpha based on interaction state
let need_click_capture = data.selected_item >= 0;
let bg_alpha: u8 = if need_click_capture { 1 } else { 0 };
target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
`

#### ✅ Pattern 2: WM_MOUSEMOVE for Hover Management

`
ust
// CORRECT: Let WM_MOUSEMOVE manage hover naturally
WM_MOUSELEAVE => {
    // Ignore - WS_EX_LAYERED triggers this unreliably
    0
}

WM_MOUSEMOVE => {
    let hit = hit_test(x, y);
    if data.hovered_item != hit {
        data.hovered_item = hit;
        render();
    }
    0
}
`

#### ✅ Pattern 3: Work With Platform Behavior

`
ust
// CORRECT: Use alpha=0 for natural pass-through
let bg_alpha = if has_selection { 1 } else { 0 };
target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
// When alpha=0, clicks naturally pass through to desktop
`

### Checklist

Before implementing layered window features:

- [ ] Understand that alpha=0 pixels bypass WM_NCHITTEST
- [ ] Don't use WM_MOUSELEAVE for hover management
- [ ] Avoid PostMessageW event forwarding
- [ ] Use state-driven alpha for click capture/pass-through
- [ ] Test hover behavior on both opaque and transparent pixels
- [ ] Verify right-click menus work correctly
- [ ] See [Windows Layered Window Guide](../guides/windows-layered-window-guide.md)

---

## Window Message Handling

### Coordinate Systems

Windows uses different coordinate systems for different messages:

| Message | Coordinates | Notes |
|---------|-------------|-------|
| WM_MOUSEMOVE | Client coordinates | Relative to window's client area |
| WM_NCHITTEST | Screen coordinates | Need to convert to client coords |
| WM_LBUTTONDOWN | Client coordinates | GET_X_LPARAM(lParam) / GET_Y_LPARAM(lParam) |

### Common Mistake: Coordinate Conversion

`
ust
// ❌ WRONG: Using screen coordinates for hit test
WM_LBUTTONDOWN => {
    let (x, y) = get_mouse_pos();  // Screen coordinates
    let hit = hit_test(x, y);  // Expects client coordinates!
}

// ✅ CORRECT: Convert to client coordinates
WM_LBUTTONDOWN => {
    let (screen_x, screen_y) = get_mouse_pos();
    let (client_x, client_y) = screen_to_client(hwnd, screen_x, screen_y);
    let hit = hit_test(client_x, client_y);
}
`

### Coordinate Conversion Helper

`
ust
fn screen_to_client(hwnd: HWND, screen_x: i32, screen_y: i32) -> (i32, i32) {
    let mut point = POINT { x: screen_x, y: screen_y };
    unsafe { ScreenToClient(hwnd, &mut point) };
    (point.x, point.y)
}
`

---

## D2D Rendering

### Premultiplied Alpha

Direct2D uses **premultiplied alpha** for layered windows:

`
ust
// Pixel format must specify premultiplied alpha
let pixel_format = D2D1_PIXEL_FORMAT {
    format: DXGI_FORMAT_B8G8R8A8_UNORM,
    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
};
`

### Color Conversion

Convert from normal RGBA to premultiplied:

`
ust
fn to_premultiplied(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8, u8) {
    let alpha = a as f32 / 255.0;
    (
        (r as f32 * alpha) as u8,
        (g as f32 * alpha) as u8,
        (b as f32 * alpha) as u8,
        a,
    )
}
`

### Alpha Blending

When drawing semi-transparent content:

`
ust
// ❌ WRONG: Normal alpha blending on premultiplied target
let brush = create_solid_brush(r, g, b, a);

// ✅ CORRECT: Premultiply the color
let (pr, pg, pb, _) = to_premultiplied(r, g, b, a);
let brush = create_solid_brush(pr, pg, pb, a);
`

---

## Window Class Registration

### CS_DBLCLKS Flag

To receive WM_LBUTTONDBLCLK messages:

`
ust
let wc = WNDCLASSEXW {
    style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,  // ← Enable double-click
    // ...
};
`

### CS_HREDRAW and CS_VREDRAW

These flags cause the entire window to redraw on resize:

`
ust
let wc = WNDCLASSEXW {
    style: CS_HREDRAW | CS_VREDRAW,  // Redraw entire window on resize
    // ...
};
`

For better performance, handle WM_SIZE manually and only redraw changed regions.

---

## DPI Awareness

### Per-Monitor DPI

Enable per-monitor DPI awareness in pp.manifest:

`xml
<dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">
    PerMonitorV2, PerMonitor
</dpiAwareness>
`

### Handling WM_DPICHANGED

`
ust
WM_DPICHANGED => {
    let new_dpi = (wParam >> 16) & 0xFFFF;
    let rect = unsafe { &*(lParam as *const RECT) };
    
    // Resize window to suggested size
    SetWindowPos(
        hwnd,
        HWND::default(),
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
    
    // Update DPI-dependent resources
    data.dpi = new_dpi;
    render();
    
    0
}
`

### DPI Scaling

Scale pixel values based on DPI:

`
ust
fn scale_for_dpi(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32) / 96
}
`

---


## Z-Order Management (Window Layering)

### Problem

WS_EX_LAYERED popup windows (WS_POPUP) without explicit Z-order management will be promoted to the top of the Z-order when clicked, potentially covering other application windows. This is undesirable for desktop organizer widgets that should stay below normal application windows.

### Z-Order Hierarchy (Top to Bottom)

`
[Other application windows]
  ← Grid windows must never appear above this level
[Active grid window]       ← Topmost among grid windows
[Other grid windows]       ← Ordered by creation time
[Desktop overlay window]   ← HWND_BOTTOM (set in overlay_window.rs)
[Desktop]
`

### Implementation Pattern

#### 1. Intercept WM_WINDOWPOSCHANGING

`
ust
// In wnd_proc.rs
WM_WINDOWPOSCHANGING => {
    if !is_zorder_suppressing() {
        let wp = unsafe { &mut *(lparam as *mut WINDOWPOS) };
        if wp.flags & SWP_NOZORDER == 0 {
            wp.hwnd_insert_after = HWND_BOTTOM;
            unsafe { PostMessageW(hwnd, WM_USER_PUSH_GRIDS_BELOW, 0, 0) };
        }
    }
    0
}
`

#### 2. Deferred Push via PostMessage

`
ust
// In wnd_proc.rs
WM_USER_PUSH_GRIDS_BELOW => {
    with_zorder_suppressed(|| push_other_grids_below(hwnd));
    0
}
`

#### 3. Recursion Prevention (Critical!)

SetWindowPos(HWND_BOTTOM) on other grid windows triggers their WM_WINDOWPOSCHANGING, which would recursively post more messages. Use a thread-local flag to suppress:

`
ust
// In zorder.rs (独立模块，从 grid_window.rs 提取)
thread_local! {
    static ZORDER_SUPPRESSING: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn is_zorder_suppressing() -> bool {
    ZORDER_SUPPRESSING.with(|f| f.get())
}

pub(crate) fn with_zorder_suppressed(f: impl FnOnce()) {
    ZORDER_SUPPRESSING.with(|flag| flag.set(true));
    f();
    ZORDER_SUPPRESSING.with(|flag| flag.set(false));
}
`

### Key Design Decisions

1. **PostMessage for deferred execution**: Avoids recursive SetWindowPos calls during WM_WINDOWPOSCHANGING processing.
2. **Thread-local flag**: Prevents exponential recursion when multiple grid windows exist.
3. **SWP_NOZORDER check**: Only intervenes when Z-order is actually changing (not during resize/move).
4. **HWND_BOTTOM**: Places grid windows below all normal application windows but above the desktop overlay.

### Checklist

Before implementing Z-order features:

- [ ] Use thread-local recursion guard (not global/static)
- [ ] Check SWP_NOZORDER flag before modifying hwndInsertAfter
- [ ] Use PostMessage for deferred Z-order adjustments
- [ ] Test with multiple grid windows (A clicked should be above B)
- [ ] Verify grid windows don't cover other applications
- [ ] Verify resize/move still works normally
- [ ] Verify drag-and-drop from overlay to grid still works (find_grid_window_at_point)
- [ ] Verify drag-and-drop between grid windows still works (find_grid_window_at_point)

### Drag-and-Drop Interaction

#### Custom Drag (SetCapture-based)

The overlay and grid windows use custom drag implementation (SetCapture + WM_MOUSEMOVE) instead of OLE DoDragDrop. This requires manual hit-testing to find target windows.

**Problem**: WindowFromPoint cannot find grid windows at HWND_BOTTOM because the overlay window (full-screen, returns HTCLIENT for all hit tests) intercepts the hit test.

**Solution**: Use ind_grid_window_at_point() which directly iterates the grid window list and checks bounding rectangles:

`
ust
// In grid_window.rs:
pub fn find_grid_window_at_point(screen_x: i32, screen_y: i32) -> HWND {
    let grids = match GRID_WINDOWS.lock() {
        Ok(g) => g,
        Err(_) => return std::ptr::null_mut(),
    };
    // Iterate from end (highest Z-order) to start
    for grid in grids.iter().rev() {
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            if GetWindowRect(grid.hwnd, &mut rect) != 0 {
                if screen_x >= rect.left && screen_x < rect.right
                    && screen_y >= rect.top && screen_y < rect.bottom
                {
                    return grid.hwnd;
                }
            }
        }
    }
    std::ptr::null_mut()
}
`

`
ust
// In overlay_wnd_proc.rs and drag_handler.rs:
// Replace: let target_hwnd = unsafe { WindowFromPoint(pt) };
// With:
let target_hwnd = crate::platform::windows::grid_window::find_grid_window_at_point(pt.x, pt.y);
`

#### OLE Drag (DoDragDrop-based)

OLE DoDragDrop is a blocking modal call. During execution, Windows performs hit-testing on all windows to find IDropTarget objects. If WM_WINDOWPOSCHANGING forces grid windows to HWND_BOTTOM during drag, the grid may end up below the overlay window, preventing OLE from triggering DragEnter/Drop on the grid's IDropTarget.

**Overlay OLE Drag-Drop Support**

The overlay window also registers as an IDropTarget to receive files dragged from Windows Explorer. This requires:

1. **bg_alpha=1**: The overlay window must have non-transparent background to receive OLE events
2. **WM_NCHITTEST**: Returns HTTRANSPARENT for blank areas to allow desktop clicks to pass through
3. **Visual feedback**: Uses the same update_overlay_drag_over_visual function as internal drags
4. **Grid positioning**: Uses dd_icon_to_overlay_with_grid to snap dropped files to the grid

**Key Implementation**:
- overlay_drop_target.rs: IDropTarget implementation for overlay window
- PendingOverlayDrop: Stores drop coordinates and insert mode state
- WM_USER_OVERLAY_DROP: Custom message to trigger drop handling in UI thread

**Solution**: Set a thread-local DRAG_IN_PROGRESS flag before DoDragDrop and clear it after. The WM_WINDOWPOSCHANGING handler skips Z-order intervention when this flag is true.

`
ust
// In ole_dnd.rs, around DoDragDrop:
super::grid_window::set_drag_in_progress(true);
let hr = unsafe { DoDragDrop(data_obj_ptr, drop_src_ptr, DROPEFFECT_MOVE, &mut effect) };
super::grid_window::set_drag_in_progress(false);
`

`
ust
// In wnd_proc.rs:
WM_WINDOWPOSCHANGING => {
    if !is_zorder_suppressing() && !is_drag_in_progress() {
        // ... force HWND_BOTTOM
    }
    0
}
`

---

## Testing

### Manual Test Checklist

For layered window features:

- [ ] Hover over opaque pixels - background highlights correctly
- [ ] Hover over transparent pixels - no flickering
- [ ] Click opaque pixels - selection works
- [ ] Click transparent pixels - clicks pass through to desktop
- [ ] Right-click opaque pixels - context menu appears
- [ ] Right-click transparent pixels - desktop context menu appears
- [ ] Resize window - content scales correctly
- [ ] Move window between monitors - DPI changes handled
- [ ] Double-click - opens item (if applicable)
- [ ] Drag and drop - works correctly (if applicable)

### Integration Tests

`
ust
#[test]
fn test_click_to_deselect() {
    let mut data = OverlayData::new();
    
    // Select an icon
    data.selected_item = Some(0);
    assert_eq!(data.selected_item, Some(0));
    
    // Click blank area (hit = -1)
    data.handle_click(-1);
    
    // Should deselect
    assert_eq!(data.selected_item, None);
    
    // Background alpha should be 0 (click-through)
    assert_eq!(data.bg_alpha, 0);
}

#[test]
fn test_hover_stability() {
    let mut data = OverlayData::new();
    
    // Hover over icon
    data.handle_mouse_move(100, 100);  // Hit test returns 0
    assert_eq!(data.hovered_item, Some(0));
    
    // Move slightly (still on same icon)
    data.handle_mouse_move(105, 105);  // Hit test still returns 0
    assert_eq!(data.hovered_item, Some(0));  // Should not change
    
    // No WM_MOUSELEAVE should have fired
    assert!(!data.mouse_leave_fired);
}
`

---

## Debug Logging

### Layered Window Debug Logs

Add these logs to diagnose issues:

`
ust
// In WM_MOUSELEAVE
WM_MOUSELEAVE => {
    log::debug!("[LAYERED] WM_MOUSELEAVE fired");
    // If this fires repeatedly, you're on alpha=0 pixel
    0
}

// In WM_MOUSEMOVE
WM_MOUSEMOVE => {
    let (x, y) = get_mouse_pos();
    let hit = hit_test(x, y);
    log::debug!("[LAYERED] WM_MOUSEMOVE ({}, {}), hit={}", x, y, hit);
    // ...
}

// In render function
fn render() {
    let bg_alpha = if data.selected_item.is_some() { 1 } else { 0 };
    log::debug!("[LAYERED] render: bg_alpha={}, selected={:?}", 
                bg_alpha, data.selected_item);
    target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
    // ...
}
`

### Log Analysis

Look for patterns in logs:

- **WM_MOUSELEAVE fires repeatedly** → You're on alpha=0 pixel, ignore this message
- **WM_MOUSEMOVE shows hit=-1 but hover still set** → Hover logic is wrong
- **No WM_LBUTTONDOWN on transparent areas** → Alpha=0 is passing through (expected)
- **WM_NCHITTEST not called on transparent areas** → Alpha=0 bypasses it (expected)

---

## Related Guides

- [Windows Layered Window Guide](../guides/windows-layered-window-guide.md) - Detailed alpha behavior
- [Cross-Layer Thinking Guide](../guides/cross-layer-thinking-guide.md) - Avoiding cross-layer bugs
- [Windows Shell API Guide](../guides/windows-shell-api-guide.md) - COM interfaces

---

## References

- [Layered Windows (Microsoft Docs)](https://docs.microsoft.com/en-us/windows/win32/winmsg/window-features#layered-windows)
- [Window Messages (Microsoft Docs)](https://docs.microsoft.com/en-us/windows/win32/winmsg/window-messages)
- [Direct2D Premultiplied Alpha](https://docs.microsoft.com/en-us/windows/win32/direct2d/premultiplied-alpha)

---

## History

This guide was created after debugging the overlay hover flickering and click-to-deselect bugs, which required 7 failed attempts before identifying the root cause as WS_EX_LAYERED pixel-level alpha behavior.

---

## Drag and Drop Implementation

### SetCapture Limitations on WS_EX_LAYERED Windows

`SetCapture` is **unstable** on WS_EX_LAYERED windows. Windows will forcibly release the capture, causing infinite WM_CAPTURECHANGED loops if you try to re-capture in the handler.

#### WRONG: SetCapture on Layered Window

```rust
// WRONG: Causes infinite WM_CAPTURECHANGED loop
fn start_drag(hwnd: HWND) {
    SetCapture(hwnd);  // Windows immediately releases this!
}

WM_CAPTURECHANGED => {
    if data.drag_active {
        SetCapture(hwnd);  // Re-capture -> infinite loop!
    }
}
```

#### CORRECT: Timer-Based Polling

```rust
// CORRECT: Use Timer to poll drag state
const DRAG_TIMER_ID: usize = 0x4F56_0001;

fn start_drag(hwnd: HWND) {
    data.drag_active = true;
    SetTimer(hwnd, DRAG_TIMER_ID, 8, null_mut()); // 8ms = 125Hz
}

fn handle_timer(hwnd: HWND, timer_id: usize) {
    if timer_id == DRAG_TIMER_ID {
        // Check state consistency
        if !data.drag_active {
            cleanup_drag(hwnd);
            return;
        }
        
        // Update drag ghost position
        update_drag_ghost();
        
        // Check if mouse released
        if !is_mouse_button_pressed(VK_LBUTTON) {
            end_drag(hwnd);
        }
    }
}

fn end_drag(hwnd: HWND) {
    data.drag_active = false;
    KillTimer(hwnd, DRAG_TIMER_ID);
    cleanup_drag_ghost();
}
```

### Drag Ghost Positioning

Drag ghost windows often have the icon centered within the window, not at the window origin. You must compensate for this offset when positioning.

#### WRONG: Ignoring Icon Offset

```rust
// WRONG: Window position != icon position
SetWindowPos(ghost_hwnd, null_mut(), cursor_x, cursor_y, 0, 0, ...);
// Icon is drawn centered, so actual icon is offset from cursor
```

#### CORRECT: Subtract Icon Offset

```rust
// CORRECT: Account for centering offset
let icon_offset_x = (window_width - icon_size) / 2;
SetWindowPos(ghost_hwnd, null_mut(), 
             cursor_x - icon_offset_x, cursor_y, 0, 0, ...);
```

### State Consistency Checking

External modules (like desktop file watchers) may reset drag state during refresh operations. Timer handlers should detect this and clean up gracefully.

#### WRONG: Blindly Trusting State

```rust
// WRONG: What if drag_active was reset externally?
fn handle_timer() {
    update_ghost();
    if !is_mouse_pressed() {
        end_drag();
    }
    // Timer keeps running even if state is inconsistent!
}
```

#### CORRECT: Validate State Consistency

```rust
// CORRECT: Check for external state changes
fn handle_timer() {
    if !data.drag_active {
        log::warn!("[DRAG] State reset externally, cleaning up");
        cleanup_ghost();
        KillTimer(hwnd, DRAG_TIMER_ID);
        return;
    }
    
    update_ghost();
    if !is_mouse_pressed() {
        end_drag();
    }
}
```

### Timer Performance Optimization

Frequent DPI checks during drag can cause lag. Cache DPI values and check less frequently.

#### WRONG: DPI Check Every Frame

```rust
// WRONG: Expensive DPI check every 8ms
fn handle_timer() {
    let dpi = get_monitor_dpi();  // System call every frame!
    update_ghost(dpi);
}
```

#### CORRECT: Cache and Check Periodically

```rust
// CORRECT: Cache DPI, check every 8 frames
static mut CACHED_DPI: u32 = 96;
static mut FRAME_COUNT: u32 = 0;

fn handle_timer() {
    FRAME_COUNT += 1;
    if FRAME_COUNT % 8 == 0 {
        CACHED_DPI = get_monitor_dpi();
    }
    update_ghost(CACHED_DPI);
}
```

### Checklist

Before implementing drag-and-drop on layered windows:

- [ ] Use Timer polling instead of SetCapture
- [ ] Set timer interval to 8ms (125Hz) for smooth tracking
- [ ] Validate state consistency in timer handler
- [ ] Account for icon centering offset when positioning ghost
- [ ] Cache DPI values to avoid performance overhead
- [ ] Clean up timer and ghost on state reset
- [ ] Test with external state modifications (file watchers, etc.)
- [ ] See [Windows Layered Window Guide](../guides/windows-layered-window-guide.md)

---


## Drag Visual Feedback

### Drag-Over State Computation

The drag-over visual feedback uses a pure function `compute_drag_over_state` to determine whether to show an insert separator or a replace highlight:

```rust
// Pure function - easily testable without HWND
fn compute_drag_over_state(
    ratio_x: f32,           // cursor X position within cell (0.0~1.0)
    col: i32, row: i32,     // grid cell coordinates
    max_cols: i32,          // grid column count
    item_at_cell: Option<i32>, // item index at target cell
    insert_threshold: f32,  // zone threshold (typically 0.30)
) -> (i32, i32, i32)       // (drag_over_item, insert_col, insert_row)
```

**Zone logic:**
- `ratio_x < threshold` → Insert mode (separator at left edge)
- `ratio_x > 1.0 - threshold` → Insert mode (separator at right edge)
- Center zone → Replace mode (highlight background)

### Shift-Insert Algorithm

When dropping in insert mode, items are shifted to make room:

```rust
// Pure function - shifts grid positions for insert
fn apply_shift_insert(
    positions: &[(i32, i32)], // current (col, row) for all items
    dragged_idx: usize,       // index of dragged item
    insert_lin: i32,          // target linear position
    cols: i32,                // grid column count
) -> Vec<(i32, i32)>         // updated positions
```

**Drag right** (drag_lin < insert_lin): shift items in `(drag_lin, insert_lin)` left by 1, place dragged at `insert_lin - 1`.

**Drag left** (drag_lin > insert_lin): shift items in `[insert_lin, drag_lin)` right by 1, place dragged at `insert_lin`.

### Cross-Box Visual Feedback

Cross-box drag-over uses `WM_USER_DRAG_OVER_UPDATE` message with cursor screen coordinates packed in LPARAM:

```rust
let lparam = ((screen_y as isize) << 16) | ((screen_x as isize) & 0xFFFF);
PostMessageW(target_hwnd, WM_USER_DRAG_OVER_UPDATE, 0, lparam);
```

### State Lifecycle

1. **Timer poll** (8ms): detect cursor position, update `drag_over_*` fields, trigger re-render
2. **Drop**: save `drag_over_insert_col/row` BEFORE clearing, pass to drop handler
3. **Drag leave**: clear all `drag_over_*` fields, re-render

### Checklist

- [ ] Use pure functions for state computation (testable without HWND)
- [ ] Save insert state before clearing in end_drag
- [ ] Use `WM_USER_DRAG_OVER_UPDATE` for cross-box visual feedback
- [ ] Clear drag-over state in WM_USER_DRAG_LEAVE handler
- [ ] Separator color: pure white `(255, 255, 255, 255)`, width 1px

## Related Files

- `src/platform/windows/drag_handler.rs` - Drag state management
- `src/platform/windows/drag_image.rs` - Drag ghost window
- `src/platform/windows/desktop_overlay/overlay_wnd_proc.rs` - Overlay drag handling
- `src/platform/windows/desktop_overlay/overlay_window.rs` - Overlay state management


## Icon Position Snapshot and Restore

### Problem

When the application exits, it restores hidden icons to the desktop via `restore_all_hidden_icons()`. 
Windows SysListView32 automatically arranges these icons, causing them to appear at incorrect positions 
instead of their original desktop locations.

### Solution

The application implements a snapshot/restore mechanism:

1. **Snapshot at startup**: After restoring hidden icons but before hiding SysListView32, 
   `AppRuntime::snapshot_desktop_positions()` captures all desktop icon positions via 
   `icon_manager::snapshot_all_desktop_positions()`.

2. **Restore at exit**: After restoring hidden icons during cleanup, 
   `AppRuntime::restore_desktop_positions()` uses `icon_manager::set_icon_position_by_name()` 
   to move icons back to their snapshot positions.

### Implementation Details

- **Snapshot storage**: Positions are stored in `AppRuntime.desktop_positions_snapshot` 
  (runtime-only, not persisted to config file).
- **Coordinate system**: Uses SysListView32 client coordinates (via `LVM_GETITEMPOSITION`).
- **Timing**: 500ms delay after `restore_all_hidden_icons()` to allow Windows to complete 
  icon restoration before attempting position correction.
- **Auto-arrange handling**: `set_icon_position_by_name()` temporarily disables desktop 
  auto-arrange, sets positions, then restores the original auto-arrange state.

### Code Flow

```
Startup:
  restore_layout()
    -> restore_all_hidden_icons()  // Restore icons from vault
    -> snapshot_desktop_positions() // Capture positions before hiding
    -> ensure_native_icons_hidden() // Hide SysListView32
    -> create overlay and boxes

Exit:
  cleanup()
    -> save config
    -> destroy boxes and overlay
    -> restore_native_icons()      // Show SysListView32
    -> restore_all_hidden_icons()  // Restore icons from vault
    -> sleep(500ms)                // Wait for Windows
    -> restore_desktop_positions() // Move icons to snapshot positions
```

### Related Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `snapshot_all_desktop_positions()` | `icon_manager.rs` | Capture all icon positions from SysListView32 |
| `set_icon_position_by_name()` | `icon_manager.rs` | Set icon position by display name |
| `AppRuntime::snapshot_desktop_positions()` | `app_runtime.rs` | Store snapshot in runtime |
| `AppRuntime::restore_desktop_positions()` | `app_runtime.rs` | Restore from snapshot |

### Checklist

- [ ] Snapshot happens AFTER restoring hidden icons (so vault icons are visible)
- [ ] Snapshot happens BEFORE hiding SysListView32 (so positions are readable)
- [ ] Restore happens AFTER restoring hidden icons during cleanup
- [ ] Sufficient delay (500ms) between restore and position correction
- [ ] Auto-arrange is temporarily disabled during position setting
- [ ] Positions are stored in runtime memory, not config file

## Shell Context Menu Result Handling (ShellMenuResult)

`shell_menu.rs` 的 `show_shell_context_menu` 使用 COM Shell API 获取右键菜单，失败时回退到自定义 `ShellExecuteW` 菜单。正确区分菜单结果至关重要。

### ShellMenuResult 语义

| 变体 | 含义 | 是否触发回退菜单 |
|------|------|:---:|
| `Cancelled` | 菜单**未能显示**（COM 创建失败、PIDL 解析失败等） | ✅ 触发 |
| `Dismissed` | 菜单**已成功显示**，用户点击关闭（TrackPopupMenu 返回 0） | ❌ 不触发 |
| `Completed(verb)` | 用户选择了菜单项并执行了操作 | ❌ 不触发 |
| `NeedsRename(path)` | 用户选择了重命名动词 | ❌ 不触发 |
| `NeedsCutMonitor(path)` | 用户选择了剪切操作 | ❌ 不触发 |
| `NeedsDelete(path)` | 用户选择了删除操作 | ❌ 不触发 |

### Common Pitfall: Cancelled vs Dismissed

`TrackPopupMenu` 返回 0 表示**菜单已显示但用户未选择任何项**（左键点击外部关闭、按 Esc 等）。这与菜单未能显示是完全不同的情况。

`
ust
// WRONG: TrackPopupMenu 返回 0 时返回 Cancelled，导致误触发回退菜单
if cmd > 0 {
    // handle selection...
} else {
    return ShellMenuResult::Cancelled;  // BUG: 关闭菜单后会弹出回退菜单!
}

// CORRECT: 区分已显示但关闭和未能显示
if cmd > 0 {
    // handle selection...
} else {
    return ShellMenuResult::Dismissed;  // 菜单已成功显示，用户关闭，无需回退
}
`

### 回退守卫模式

`
ust
pub fn show_shell_context_menu(hwnd: HWND, file_path: &str) -> ShellMenuResult {
    let result = try_windows_crate_context_menu(hwnd, &menu_path);
    if should_skip_fallback(&result) {
        return result;
    }
    // 仅在 Cancelled 时回退
    show_shell_fallback_menu(hwnd, &menu_path)
}

/// 纯函数：判断是否跳过回退菜单
fn should_skip_fallback(result: &ShellMenuResult) -> bool {
    !matches!(result, ShellMenuResult::Cancelled)
}
`


### Timer-Based Hover Management for Layered Windows

When overlay windows use dynamic alpha (alpha=0 for blank areas, alpha=1 for selected),
WM_MOUSEMOVE is **not delivered** when the cursor moves from an icon (opaque pixel) to
a blank area (transparent pixel). This means hovered_item cannot be cleared through
the normal mouse-move path.

#### Problem

`
┌──────────────────────────────┐
│  Overlay (WS_EX_LAYERED)     │
│  ┌──────────┐                │
│  │  Icon    │ ← alpha=1, WM_MOUSEMOVE works │
│  │  (hover) │                │
│  └──────────┘                │
│                              │
│  (blank, alpha=0)            │ ← WM_MOUSEMOVE NOT delivered!
│                              │
└──────────────────────────────┘
`

The cursor moves from icon → blank, but no WM_MOUSEMOVE arrives.
hovered_item stays set, hover highlight persists visually.

#### Correct Pattern: Polling Timer

`
ust
// In handle_mouse_move: when an icon becomes hovered, start timer
if data.hovered_item >= 0 {
    SetTimer(hwnd, TIMER_ID_OVERLAY_HOVER, 50, std::ptr::null_mut());
}

// In WM_TIMER handler: poll cursor position
WM_TIMER => {
    if wparam == TIMER_ID_OVERLAY_HOVER {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let (local_x, local_y) = screen_to_overlay_client(hwnd, pt.x, pt.y);
        let hit = overlay_hit_test(data, local_x, local_y);
        if data.hovered_item != hit {
            data.hovered_item = hit;
            render();
        }
        // Kill timer when no hover and no selection
        if data.hovered_item < 0 && data.selected_item < 0 {
            KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
        }
    }
}

// In WM_DESTROY: kill timer
WM_DESTROY => {
    KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
}

// In deselect path (click blank): kill timer + clear hover
data.hovered_item = -1;
KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
`

#### Timer Interval

- **50ms** (20 FPS): Good balance between responsiveness and CPU usage
- Too fast (8ms): Unnecessary CPU usage for non-drag hover tracking
- Too slow (200ms): Noticeable lag when cursor leaves icon area

#### Related Constants

- TIMER_ID_OVERLAY_HOVER (defined in fi_types.rs)

### Related Files

- `src/platform/windows/shell_menu.rs` — ShellMenuResult 枚举 + show_shell_context_menu + should_skip_fallback
- `src/platform/windows/wnd_proc.rs` — handle_shell_menu_result (分类框)
- `src/platform/windows/desktop_overlay/overlay_wnd_proc.rs` — handle_overlay_shell_menu_result (覆盖窗口)

### Checklist

- [ ] TrackPopupMenu 返回 0 时使用 `Dismissed` 而非 `Cancelled`
- [ ] 回退守卫仅对 `Cancelled` 触发回退菜单
- [ ] `handle_shell_menu_result` 和 `handle_overlay_shell_menu_result` 的 match 覆盖 `Dismissed` 分支

---

## Resize Edge Hover Indicator

分类框边缘拖拽 resize 时，鼠标悬停的边缘会显示半透明指示条，提示用户可拖拽调整尺寸。

### 圆角适配

分类框使用 `D2D1_ROUNDED_RECT` 绘制圆角背景（默认 radius=16），边缘指示条需要避开圆角区域，避免直角矩形突出于圆角背景的视觉冲突。

### 实现方式

使用纯函数 `compute_edge_rect()` 计算边缘条矩形，根据 `hover_edge` 值返回避开圆角的坐标：

```rust
pub fn compute_edge_rect(
    hover_edge: i32,      // 0=无, 1=左, 2=右, 3=上, 4=下
    window_width: f32,
    window_height: f32,
    border_width: f32,
    corner_radius: f32,
) -> (f32, f32, f32, f32) // (left, top, right, bottom)
```

**边缘条避让规则**：
- 左/右边缘：top 起点 ≥ corner_radius，bottom 终点 ≤ height - corner_radius
- 上/下边缘：left 起点 ≥ corner_radius，right 终点 ≤ width - corner_radius

### 相关文件

- `src/platform/windows/d2d_render.rs` — `compute_edge_rect()` 函数及渲染逻辑
- `src/platform/windows/wnd_proc.rs` — `WM_NCHITTEST` 中 `hover_edge` 状态更新

### 单元测试

`d2d_render.rs` 中包含 11 个单元测试覆盖：
- 四个方向边缘的矩形计算
- 无边缘 / 无效边缘 / 负边缘值
- 边界约束验证
- 圆角避让验证
- DPI 缩放兼容性
- 小窗口场景

## Inline Rename (Async EDIT Control)

### Problem

The grid window's message loop does not dispatch Windows messages to child controls. A synchronous EDIT control
created on the message thread blocks message processing and cannot receive keyboard input (especially IME
for CJK characters).

### Solution

Run the EDIT control on a **dedicated background thread** with its own standard Windows message loop.
Communicate results back to the parent window via PostMessage with custom WM_USER_* messages.

### Architecture

`
Parent Window (grid/overlay)
  │
  ├── show_rename_editor_async()      # Spawns background thread
  │     │
  │     └── edit_thread_main()        # Creates STATIC container + EDIT child
  │           │
  │           ├── edit_subclass()     # Handles Enter/Escape/KillFocus
  │           │     ├── VK_RETURN → PostQuitMessage(0)
  │           │     ├── VK_ESCAPE → EDIT_RESULT = Cancel
  │           │     └── WM_KILLFOCUS → deferred focus check
  │           │
  │           └── cleanup_and_notify() # Sends WM_USER_RENAME_RESULT or WM_USER_RENAME_CANCEL
  │
  ├── WM_USER_RENAME_RESULT handler   # Performs actual file rename
  │     ├── build_rename_filename()   # Preserves file extension
  │     ├── icon_hider::rename_icon_file()
  │     └── display_name_from_path()  # Updates display name
  │
  └── WM_USER_RENAME_CANCEL handler   # Clears renaming_item_index
`

### Key Implementation Details

1. **Thread safety**: EDIT_THREAD_ACTIVE AtomicBool prevents concurrent rename editors
2. **Extension preservation**: uild_rename_filename() always preserves original extension;
   the EDIT control pre-selects only the filename stem via EM_SETSEL
3. **Label hiding**: During rename, 
enaming_item_index tells the renderer to skip
   drawing the icon label (the EDIT control shows it instead)
4. **Focus-lost handling**: WM_KILLFOCUS posts a deferred WM_USER_QUIT_EDIT message
   to check if focus truly left the EDIT (allows quick re-click)
5. **Font consistency**: EDIT uses the same "Microsoft YaHei UI" font as icon labels,
   scaled to the parent window's DPI

### Shared Pure Functions (in rename_editor.rs)

| Function | Purpose | Testable |
|----------|---------|----------|
| uild_rename_filename(new_name, old_path) | Build final filename preserving extension | ✅ 9 tests |
| display_name_from_path(file_path) | Extract display name (stem) from path | ✅ 6 tests |
| get_name_stem_len(name) | Calculate stem length for EM_SETSEL | ✅ 8 tests |

### Checklist

- [ ] EDIT control runs on its own thread (not main thread)
- [ ] 
enaming_item_index set before showing editor, cleared on result/cancel
- [ ] Extension always preserved (user edits stem only)
- [ ] Label hidden during rename (rendering checks 
enaming_item_index)
- [ ] Font matches icon label font (Microsoft YaHei UI, DPI-scaled)
- [ ] EDIT_THREAD_ACTIVE prevents concurrent editors
- [ ] Result communicated via PostMessage (not blocking calls)


### Timer-Based Hover Management for Layered Windows

When overlay windows use dynamic alpha (alpha=0 for blank areas, alpha=1 for selected),
WM_MOUSEMOVE is **not delivered** when the cursor moves from an icon (opaque pixel) to
a blank area (transparent pixel). This means hovered_item cannot be cleared through
the normal mouse-move path.

#### Problem

`
┌──────────────────────────────┐
│  Overlay (WS_EX_LAYERED)     │
│  ┌──────────┐                │
│  │  Icon    │ ← alpha=1, WM_MOUSEMOVE works │
│  │  (hover) │                │
│  └──────────┘                │
│                              │
│  (blank, alpha=0)            │ ← WM_MOUSEMOVE NOT delivered!
│                              │
└──────────────────────────────┘
`

The cursor moves from icon → blank, but no WM_MOUSEMOVE arrives.
hovered_item stays set, hover highlight persists visually.

#### Correct Pattern: Polling Timer

`
ust
// In handle_mouse_move: when an icon becomes hovered, start timer
if data.hovered_item >= 0 {
    SetTimer(hwnd, TIMER_ID_OVERLAY_HOVER, 50, std::ptr::null_mut());
}

// In WM_TIMER handler: poll cursor position
WM_TIMER => {
    if wparam == TIMER_ID_OVERLAY_HOVER {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let (local_x, local_y) = screen_to_overlay_client(hwnd, pt.x, pt.y);
        let hit = overlay_hit_test(data, local_x, local_y);
        if data.hovered_item != hit {
            data.hovered_item = hit;
            render();
        }
        // Kill timer when no hover and no selection
        if data.hovered_item < 0 && data.selected_item < 0 {
            KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
        }
    }
}

// In WM_DESTROY: kill timer
WM_DESTROY => {
    KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
}

// In deselect path (click blank): kill timer + clear hover
data.hovered_item = -1;
KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
`

#### Timer Interval

- **50ms** (20 FPS): Good balance between responsiveness and CPU usage
- Too fast (8ms): Unnecessary CPU usage for non-drag hover tracking
- Too slow (200ms): Noticeable lag when cursor leaves icon area

#### Related Constants

- TIMER_ID_OVERLAY_HOVER (defined in fi_types.rs)

### Related Files

- src/platform/windows/rename_editor.rs — Thread-based EDIT control + shared helpers
- src/platform/windows/wnd_proc.rs — Grid window rename result handling
- src/platform/windows/desktop_overlay/overlay_wnd_proc.rs — Overlay rename result handling
- src/platform/windows/d2d_render.rs — Label hiding during rename
- src/platform/windows/desktop_overlay/overlay_render.rs — Overlay label hiding during rename

## OLE Drag-to-Explorer Pattern

### Problem

分类框和覆盖区域需要支持拖拽文件到外部窗口（资源管理器/Explorer）。
由于应用使用 WS_EX_LAYERED 窗口和自定义拖拽（Timer 轮询替代 SetCapture），
标准 OLE 拖拽无法直接启动，需要在检测到 Explorer 窗口时手动触发 DoDragDrop。

### Solution

在拖拽定时器轮询中检测外部窗口（ind_external_window_at_point），
当光标悬停在 Explorer（CabinetWClass）窗口上时，启动 OLE 拖拽。

### Architecture

`
拖拽定时器（handle_grid_drag_timer / handle_overlay_drag_timer）
  │
  ├── 光标在分类框上 → 同框/跨框拖拽可视化
  ├── 光标不在任何分类框上
  │     │
  │     └── find_external_window_at_point()
  │           │
  │           ├── 找到 CabinetWClass → start_*_ole_drag()
  │           │     ├── 停止定时器 + 重置拖拽状态
  │           │     ├── 销毁拖拽残影
  │           │     ├── 构造 IkunBoxDragData（CF_HDROP + 自定义格式）
  │           │     ├── 隐藏覆盖窗口和源窗口（让 OLE hit-test 到达 Explorer）
  │           │     ├── ole_dnd::start_box_drag() → DoDragDrop()
  │           │     ├── 恢复窗口可见性
  │           │     └── 重新渲染（图标保持原位，复制操作）
  │           │
  │           └── 未找到 → 显示覆盖区域拖拽可视化
  └── 鼠标释放 → end_*_drag()
`

### Key Implementation Details

1. **窗口隐藏**：DoDragDrop 是阻塞调用，期间需要 Explorer 能接收到 OLE hit-test。
   覆盖窗口是全屏分层窗口，会拦截 hit-test，必须先 ShowWindow(SW_HIDE)。
   分类框也需隐藏以确保 OLE 正确检测。拖拽完成后用 SW_SHOWNA 恢复。

2. **set_drag_in_progress(true)**：DoDragDrop 期间禁止 Z-order 干预，
   否则分类框被 WM_WINDOWPOSCHANGING 强制 HWND_BOTTOM 后可能低于 overlay。

3. **复制操作**：拖拽到 Explorer 始终使用 DROPEFFECT_COPY，图标保持原位不删除。

4. **ind_external_window_at_point**：使用 EnumWindows 遍历顶层窗口，
   仅匹配 CabinetWClass（Explorer 窗口类名），跳过本应用窗口。


### Timer-Based Hover Management for Layered Windows

When overlay windows use dynamic alpha (alpha=0 for blank areas, alpha=1 for selected),
WM_MOUSEMOVE is **not delivered** when the cursor moves from an icon (opaque pixel) to
a blank area (transparent pixel). This means hovered_item cannot be cleared through
the normal mouse-move path.

#### Problem

`
┌──────────────────────────────┐
│  Overlay (WS_EX_LAYERED)     │
│  ┌──────────┐                │
│  │  Icon    │ ← alpha=1, WM_MOUSEMOVE works │
│  │  (hover) │                │
│  └──────────┘                │
│                              │
│  (blank, alpha=0)            │ ← WM_MOUSEMOVE NOT delivered!
│                              │
└──────────────────────────────┘
`

The cursor moves from icon → blank, but no WM_MOUSEMOVE arrives.
hovered_item stays set, hover highlight persists visually.

#### Correct Pattern: Polling Timer

`
ust
// In handle_mouse_move: when an icon becomes hovered, start timer
if data.hovered_item >= 0 {
    SetTimer(hwnd, TIMER_ID_OVERLAY_HOVER, 50, std::ptr::null_mut());
}

// In WM_TIMER handler: poll cursor position
WM_TIMER => {
    if wparam == TIMER_ID_OVERLAY_HOVER {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let (local_x, local_y) = screen_to_overlay_client(hwnd, pt.x, pt.y);
        let hit = overlay_hit_test(data, local_x, local_y);
        if data.hovered_item != hit {
            data.hovered_item = hit;
            render();
        }
        // Kill timer when no hover and no selection
        if data.hovered_item < 0 && data.selected_item < 0 {
            KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
        }
    }
}

// In WM_DESTROY: kill timer
WM_DESTROY => {
    KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
}

// In deselect path (click blank): kill timer + clear hover
data.hovered_item = -1;
KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
`

#### Timer Interval

- **50ms** (20 FPS): Good balance between responsiveness and CPU usage
- Too fast (8ms): Unnecessary CPU usage for non-drag hover tracking
- Too slow (200ms): Noticeable lag when cursor leaves icon area

#### Related Constants

- TIMER_ID_OVERLAY_HOVER (defined in fi_types.rs)

### Related Files

| File | Role |
|------|------|
| drag_handler.rs | 分类框拖拽协调器（start_grid_ole_drag） |
| desktop_overlay/overlay_wnd_proc.rs | 覆盖区域拖拽协调器（start_overlay_ole_drag） |
| zorder.rs | ind_external_window_at_point + set_drag_in_progress |
| ole_dnd.rs | OLE 拖拽源（start_box_drag / DoDragDrop） |

### Checklist

- [ ] 拖拽定时器中检测外部窗口（ind_external_window_at_point）
- [ ] OLE 拖拽前隐藏覆盖窗口和源窗口
- [ ] OLE 拖拽后恢复窗口可见性（SW_SHOWNA）
- [ ] 使用 DROPEFFECT_COPY（复制操作，图标保持原位）
- [ ] set_drag_in_progress(true) 包裹 DoDragDrop 调用
- [ ] 拖拽完成后重新渲染源窗口

---

## Icon and Thumbnail Extraction

### IShellItemImageFactory for Thumbnails

Windows Explorer displays file thumbnails using `IShellItemImageFactory::GetImage()`. This API returns actual file content previews (images, video frames, PDF first pages) rather than generic file type icons.

#### Correct Pattern: Use windows crate Safe API

```rust
// CORRECT: Use windows crate IShellItemImageFactory
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT};
use windows::Win32::Foundation::SIZE;
use windows::core::PCWSTR;

let factory: IShellItemImageFactory = SHCreateItemFromParsingName(pcwstr, None)?;
let size = SIZE { cx: 256, cy: 256 };
let hbitmap = factory.GetImage(size, SIIGBF_RESIZETOFIT)?;
```

#### Wrong: Manual COM vtable

```rust
// WRONG: Manual vtable definition (violates quality-guidelines.md Forbidden Pattern #6)
#[repr(C)]
struct IShellItemImageFactoryVtbl {
    query_interface: unsafe extern "system" fn(...) -> HRESULT,
    get_image: unsafe extern "system" fn(...) -> HRESULT,
    // ... manual vtable
}
```

#### Key Flags

- `SIIGBF_RESIZETOFIT`: Generate thumbnail on-demand if not cached
- `SIIGBF_THUMBNAILONLY`: Only return cached thumbnails (returns error if not cached)

**Recommendation**: Use `SIIGBF_RESIZETOFIT` without `THUMBNAILONLY` for content files that should display a preview.

#### Source Selection: Content Preview vs Application Icon

`IShellItemImageFactory` is not a universal first choice. For `.lnk`, `.exe`, `.dll`,
`.msi`, `.cmd`, `.bat`, `.com`, `.url`, and `.appref-ms`, Shell may return a small
application glyph centered in a large thumbnail canvas. Scaling that canvas into a grid
cell makes the visible icon much smaller than neighboring icons.

Use `should_extract_thumbnail()` in `src/platform/windows/icon_extract.rs` as the single
selection policy:

- Content files: try `IShellItemImageFactory`, then fall back to Shell icon extraction.
- Application/shortcut files: skip thumbnails and use `SHDefExtractIconW`/`SHGetFileInfoW`.
- `.lnk`: resolve the target first, then apply the target file's extraction policy.

Do not duplicate extension checks at call sites. Add table-driven unit tests whenever the
application-extension set changes, including uppercase extensions.

#### Conversion to HICON

`IShellItemImageFactory` returns `HBITMAP` (premultiplied alpha). Convert to `HICON` for compatibility with existing rendering pipeline:

```rust
fn thumbnail_to_hicon(hbitmap: RawHBITMAP) -> HICON {
    // Get bitmap dimensions via GetDIBits
    // Create monochrome mask bitmap (all zeros = opaque)
    // Use CreateIconIndirect to combine color + mask
    // Release original HBITMAP
}
```

#### Transparent Canvas Normalization

HICON sources can contain a valid glyph surrounded by transparent pixels. Use the pure
helpers in `src/platform/windows/icon_bitmap.rs` to calculate non-zero-alpha bounds and
the aspect-ratio-preserving destination rectangle. `d2d_render.rs` applies these helpers
once when an HICON is converted to a cached D2D bitmap.

This normalization belongs in the shared D2D icon path so the classification grid,
desktop overlay, and drag image stay visually consistent. It must not change
`ViewParams::icon_size`, grid cell dimensions, or label coordinates.

Required pure-logic tests:

- transparent padding is removed;
- full-canvas and fully transparent inputs are handled;
- malformed dimensions do not panic;
- landscape and portrait content remain centered without stretching.

### DWRITE Text Trimming

When truncating long file names in icon labels, use `DWRITE_TRIMMING_GRANULARITY_CHARACTER` instead of `WORD`:

```rust
// CORRECT: Character-level trimming for file names
let trimming = DWRITE_TRIMMING {
    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    delimiter: 0,
    delimiterCount: 0,
};

// WRONG: Word-level trimming treats entire filename as one word
let trimming = DWRITE_TRIMMING {
    granularity: DWRITE_TRIMMING_GRANULARITY_WORD,  // "20260729-105446" -> "2..."
    // ...
};
```

**Reason**: File names without spaces (like `20260729-105446.png`) are treated as single words by DirectWrite, causing aggressive truncation.


### Timer-Based Hover Management for Layered Windows

When overlay windows use dynamic alpha (alpha=0 for blank areas, alpha=1 for selected),
WM_MOUSEMOVE is **not delivered** when the cursor moves from an icon (opaque pixel) to
a blank area (transparent pixel). This means hovered_item cannot be cleared through
the normal mouse-move path.

#### Problem

`
┌──────────────────────────────┐
│  Overlay (WS_EX_LAYERED)     │
│  ┌──────────┐                │
│  │  Icon    │ ← alpha=1, WM_MOUSEMOVE works │
│  │  (hover) │                │
│  └──────────┘                │
│                              │
│  (blank, alpha=0)            │ ← WM_MOUSEMOVE NOT delivered!
│                              │
└──────────────────────────────┘
`

The cursor moves from icon → blank, but no WM_MOUSEMOVE arrives.
hovered_item stays set, hover highlight persists visually.

#### Correct Pattern: Polling Timer

`
ust
// In handle_mouse_move: when an icon becomes hovered, start timer
if data.hovered_item >= 0 {
    SetTimer(hwnd, TIMER_ID_OVERLAY_HOVER, 50, std::ptr::null_mut());
}

// In WM_TIMER handler: poll cursor position
WM_TIMER => {
    if wparam == TIMER_ID_OVERLAY_HOVER {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let (local_x, local_y) = screen_to_overlay_client(hwnd, pt.x, pt.y);
        let hit = overlay_hit_test(data, local_x, local_y);
        if data.hovered_item != hit {
            data.hovered_item = hit;
            render();
        }
        // Kill timer when no hover and no selection
        if data.hovered_item < 0 && data.selected_item < 0 {
            KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
        }
    }
}

// In WM_DESTROY: kill timer
WM_DESTROY => {
    KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
}

// In deselect path (click blank): kill timer + clear hover
data.hovered_item = -1;
KillTimer(hwnd, TIMER_ID_OVERLAY_HOVER);
`

#### Timer Interval

- **50ms** (20 FPS): Good balance between responsiveness and CPU usage
- Too fast (8ms): Unnecessary CPU usage for non-drag hover tracking
- Too slow (200ms): Noticeable lag when cursor leaves icon area

#### Related Constants

- TIMER_ID_OVERLAY_HOVER (defined in fi_types.rs)

### Related Files

- `src/platform/windows/icon_extract.rs` -- Thumbnail extraction + HICON conversion
- `src/platform/windows/d2d_render.rs` -- Text trimming for icon labels

## 多显示器拖拽边界与双击保护（2026-11-26）

> 用户报「分类框拖到主屏/副屏过渡时显示异常、有时整块漂出屏幕」
> 「双击图标有时候没反应」。两条都属于**跨层几何/输入契约**问题，改任何一处前先看本节。

### 玻璃块是单屏模型（不可绕过的事实）

liquid-verre-rs 的块只归属**中心所在那一片画布**，并按 `策略 ①` 不做跨屏大玻璃：

- `core::block::rehome_block`：中心跨屏 → 换屏 + 按两屏 DPI 比例重采样宽高，
  并把块**中心**钳制在该屏内（即允许一半悬在屏外）；
- 每块屏一张整屏画布（`SetWindowRgn` 裁剪）→ 超出归属屏的那半块**根本不渲染**。

后果：分类框停在跨屏位置时，玻璃只显示一半，而代理窗口占满两块屏
—— 观感就是「一半主屏一半副屏显示异常」，且此时点击落点与块不一致。

### 宿主的边界职责（口径固定，勿再发明）

| 时机 | 动作 | 实现 |
|------|------|------|
| 建窗 / 恢复布局 | 单屏吸附 | `grid_window::create_grid_window` |
| 引擎模式标题拖拽 | 单屏吸附（**唯一真实源头**） | `wnd_proc` 的 `WM_WINDOWPOSCHANGED` |
| 回退模式模态拖拽 | 单屏吸附 | `wnd_proc` 的 `WM_MOVING` |
| 布局落盘 | 非法位置吸附后再写配置 | `grid_window::collect_grid_layout` |
| 库会话跟随中 | **只夹虚拟桌面**，镜像块矩形 | `grid_window::follow_glass_session` |
| 库会话收尾 | 块与窗口同屏一致性纠偏 | `grid_window::end_glass_resize_session` |

强制口径：

- 纯算术一律下沉 `src/box_core/monitor_geom.rs`（0 unsafe、可单测）：
  `clamp_to_bounds` / `monitor_index_for_center` / `snap_into_single_monitor` /
  `rect_fits_any_monitor`；平台层只做 `RECT` ↔ 数组的薄转换。
- **引擎模式下 `WM_MOVING` 不会触发**（标题拖拽走本窗口 `SetWindowPos`），
  只在 `WM_MOVING` 加边界 = 等于没加（本次踩坑点）。
- 吸附必须**幂等**（只改位置不改尺寸），且**绝不**在库会话期间把块往回拽：
  会话期块矩形是几何真源，宿主反向 push 会污染库的会话锚点。
- 归属屏按**块中心**判定（与库 `rehome_block` 同构），宿主另起一套判据必然与库漂移。
- `collect_grid_layout` 会被 Tauri 命令线程调用：此处**禁止**跨线程 `SetWindowPos`
  / 渲染（历史前科：卡死 WebView2 事件循环），只允许吸附要写进配置的值。

### DPI 比例以块为准

`glass_engine` 宿主侧 `entry.ratio` 只在 push 时按 `GetDpiForWindow` 更新，
跨屏过渡那一拍代理窗 DPI 常常还是旧屏值 → 内容窗口/内容纹理按旧 DPI 尺寸落位。

- 回读块矩形必须用 `snapshot.scale`（块自己的权威比例），不要用 `entry.ratio`；
- 跟随 tick 里检测 `block_ratio_drifted` → `sync_block_ratio` +
  作废 `content_valid` / `submitted_glass` 强制全量重绘（200ms 去抖）。
- `WM_DPICHANGED`：系统建议矩形要在**进入 handler 时**一次性读干净
  （`RECT` 非 `Copy`，且 handler 返回后 `lparam` 指向的临时结构即失效，
  在 `SetWindowPos` 之后再解引用是 UB）；DPI 变大导致物理尺寸超过落位屏时，
  用 `monitor_utils::fit_content_rect_into_monitor` 兜底（必要时允许缩小）。

### 图标拖拽不得抢走双击

- 拖拽启动阈值必须按窗口 DPI 折算：
  `drag_logic::drag_threshold_physical_px(GetSystemMetrics(SM_C?DRAG), dpi)`，
  夹在 `[5, 16]`。硬编码 `DRAG_THRESHOLD = 5` 物理像素在高 DPI 下等于手抖即拖。
- `DoDragDrop` **阻塞本框消息线程**，且起拖时会给本框加 `WS_EX_TRANSPARENT`。
  分类框常驻 `HWND_BOTTOM`，资源管理器窗口几何上盖住它时，
  「光标落在 Explorer 矩形内」不代表用户把图标拖出了框 ——
  必须再判一条「光标已离开 `GridWindowData::drag_box_rect`」才允许升级 OLE。
- `WM_LBUTTONDBLCLK` 开头必须先收尾进行中的自定义拖拽
  （`drag_pending`/`drag_active` + `KillTimer(TIMER_ID_GRID_DRAG)` + 销毁残影），
  否则残影/OLE 模态循环会吞掉双击，表现为「有时候没反应」。

### 库逻辑像素口径：最小尺寸禁止再按窗口 DPI 换算（2026-11-26）

库（liquid-verre-rs）的坐标单位契约：

- `scale = 该屏 DPI / 主屏 DPI`（**主屏 DPI，不是 96**）；
- 块**物理**尺寸 = 逻辑尺寸 × `scale`；逻辑像素 = 主屏物理像素；
- `ResizeOptions.min_width / min_height` 是**逻辑像素**硬下限
  （`input::drag::ResizeState::apply` 里 `w.max(min_w)`，夹住后
  `x = x0 + (w0 - w)` 会反向挪左边缘）。

宿主侧 `monitor_dpi_ratio(dpi)` 已经是同一个 scale，因此：

```text
正确： min_logical = 期望物理尺寸(dpi) / scale   = scale_for_dpi(v, dpi) / ratio
错误： 只在「主屏恰好 96 DPI」时等于 v
```

本机实测主屏 192 DPI、副屏 96 DPI（ratio=0.5）：旧写法得到
`scale_for_dpi(200,96)/0.5 = 400` **逻辑**像素 = 400 物理像素，
比历史最小框（355/377）还宽 → 副屏上每个框天生低于硬下限，直接产出两个可见故障：

1. 拖左边缘 → 宽度立刻被抬到 400、锚定式把左边缘外扩 → **右边缘持续向右漂移**；
2. 尺寸被强改 → 列数变化 → 图标重排 →「位置变乱」。

**定案**：`glass_resize_floor` 一律返回常量逻辑值（200 x 140，与
`WM_GETMINMAXINFO` 的 96-DPI 基线同源），**不接受窗口 DPI 参与换算**；
单测同时钉「与 dpi 无关」与「不得被 DPI 二次放大（<=240x180）」。

判断任何「跨屏尺寸跳变」问题时，先把这三条单位链写清楚再动手：
`GetDpiForWindow` → `monitor_dpi_ratio` → 库 `snapshot.scale` → 块物理尺寸。

### 副屏缩放「锚定边漂移」是 `liquid-verre-rs` 的缺陷，宿主不要纠正（2026-11-26 定案）

副屏（100%）+ 主屏（200%）一次左缘拖拽实测（`RUST_LOG=info` 块矩形）：

```text
光标    3139 -> 3339  (+200)
块左缘  3139 -> 3339  (+200)   正确：跟手
块右缘  3637 -> 3737  (+100)   错误：锚定边本该为 0
```

规律：**左缘位移 = δ，宽度变化 = δ x scale**，锚定边位移 = δ(1 - scale)。
主屏 scale = 1 位移为 0（用户报告「主屏没有问题」），副屏 scale = 0.5 每 200px 漂 100px，
与两轮实测数据完全吻合。

根因在库（0.5.4 源码已定位，完整取证与补丁见
`.trellis/tasks/11-26-fix-cross-screen-icon-scramble-and-resize-drift/liquid-verre-rs-defect.md`）：

1. `src/input/drag.rs::ResizeState::apply` 里 `x = x0 + (w0 - w)`（以及 `y = y0 + (h0 - h)`）
   把**逻辑像素**的宽度增量直接加到**物理像素**的 `x` 上 —— `origin.x/y` 是虚拟屏物理像素、
   `origin.w/h` 是逻辑像素（库自己的注释就是这么写的），量纲不一致，只在 scale = 1 时侥幸成立；
2. `src/runtime/mod.rs` 每帧的 `drag_scale` 取 `app.scale_of(b.monitor)`，而会话原点
   （`src/platform/window.rs`）与快照物理尺寸用的是 `b.scale`；`App::scale_of` 在下标越界时
   `map_or(1.0, ..)` 静默返回 1.0。两套 scale 一旦不一致，缩放算术立刻错位（本机实测即此形态）。

**宿主侧无法修复**（下列三条都试过，已全部回退）：`ResizeState` / `DragSession` 是
`pub(crate)`，宿主拿不到也改不了会话原点；库每帧都用**会话开始时记录的 origin** 重算并整块
覆盖 `b.x/b.y/b.width`，所以宿主中途 `set_rect` 回推下一帧就被抹掉；只夹代理窗口等于没修，
因为可见的是**库画的玻璃块**，结果变成「玻璃走、内容不走」的双层错位。因此：

1. 缩放/移动会话期间**禁止**按宿主判据改写块几何（锚点回推、夹对侧边这类补偿一律不许）；
   曾实现过的 `glass_resize_anchor` / `apply_resize_anchor` / `session_content_rect` 已于
   2026-11-26 删除，**不要往回加**。
2. 会话期宿主只做**虚拟桌面边界夹取**（`clamp_content_to_desktop`）；单屏归位留到
   `end_glass_resize_session` 收尾一次性完成；`WM_WINDOWPOSCHANGED` 的反向下发在本地会话
   结束后保留 300ms 宽限期（`GridWindowData::glass_session_ended_at`），避免用落位中的
   陈旧窗口矩形去覆盖库的会话原点。
3. 根治只有两条路：修库并发版（首选）；或临时把 `box_style.liquid_glass.resize.enabled`
   关掉改由宿主驱动缩放（实测掉到约 45-54Hz、失去库的把手/边缘反馈，需产品确认后才做）。

判据日志（`RUST_LOG=info`）：`[GLASS-PUSH] phys=.. -> logical=.. 库当前=[..]`、
`[GLASS-DIAG] .. glass: logical=[..] scale=.. physW=.. hostRatio=..`。
排查「拖边缘时某条边不钉住」：看 `x + w x scale` 是否守恒；不守恒 = 库缺陷，不要写宿主补偿。

**0.5.5 落地记录（2026-11-26 13:08）**：库侧按本缺陷报告根修并发布 `liquid-verre-rs 0.5.5`
（commit `14751faf`，tag `v0.5.5`），修法是**缩放会话全程物理域计算**（比报告 5.1 的
「补一个 `* s`」更彻底：`origin` 改存按下时刻物理矩形、`dx/dy` 不再 `/ s`、返回 `/ s`
与写回 `* s` 相消），并补了 4 条判别单测（0.5 / 2.0 锚定零漂移、会话中途 scale 翻转
物理不变、min-clamp 半 scale 锚定精确 —— 旧实现逐条必红）。宿主已升级
`=0.5.4 -> =0.5.5`，`cargo check/test/clippy/fmt` 全绿且适配层**零代码改动**（新契约见
`spec/guides/tauri-window-contract.md`「缩放会话量纲契约」）。本节第 1 条禁令仍然有效：
**库会话期内宿主不得改写块几何**。
### 图标重排的「旧列数」必须记账，不能靠猜

`box_core::layout::reflow_positions(positions, new_cols, old_cols_hint)`：

- 旧实现用 `max(grid_col)+1` 当旧列数 —— 框里只要有空洞（删过图标 / 手动留空位 /
  最后一行不满）就**必然偏小**（列数变多时又必然偏大），线性重映射随之整体错位，
  且**不可逆**（一次错位永久留在配置里）；
- 现在由 `GridWindowData::layout_cols` 在每轮 `recalculate_layout` 记下真实生效列数，
  `reflow_items` 直接采信（**不与本轮列数取较大**，否则「列数变多」会被误判成没变）；
  `layout_cols <= 0` 才退回推断，兼容历史数据；
- 两条短路：无固定位项 → 不动；`old_cols == new_cols` → 不动
  （跨屏 DPI 重采样后逻辑尺寸不变，列数通常不变，任何一次多余重排都是净伤害）。

### 内容排布与滚动：列数是唯一的换行真源（2026-11-26 需求 2.1 / 2.2）

四条纪律，全部有单测钉住：

1. **先重算列数，再重排固定位。** 唯一入口 `grid_layout::relayout_for_size(data, w, h, dpi)`：
   进入时先抄 `prev = data.layout_cols`，`recalculate_layout` 之后再 `reflow_to(data, prev)`。
   顺序颠倒（旧代码在 `WM_SIZE` 里正是 reflow 先跑）会让 `cols == layout_cols` 恒成立，
   `reflow_positions` 判成「列数没变」直接早退 -> 缩小宽度时固定位留在旧列上，
   图标直接画到框外（用户报「缩小不换行」的真根因，与库无关）。
2. **网格留白/间隙必须 DPI 化。** `calculate_grid` 的 `padding` 必须走
   `layout_calc::grid_side_pad(dpi)` / `grid_top_gap(dpi)`；写死 8px 会让
   `cols = (W - 16) / cell_w` 的分子不是 DPI 同比量，同一只框在主屏(192)与副屏(96)
   会差出整整一列 -> 「拖到另一块屏图标就乱」。DPI 化后**列数对 DPI 恒定**。
3. **跨屏只允许吸收取整噪声。** DPI 重采样走 `relayout_for_dpi`，它用
   `snap_width_for_cols` 把内容宽度吸回「上一轮列数」区间，容差只有 ±2x留白；
   用户真的把框缩小/放大超出容差时**必须放行**，否则需求 2.1 的自动换行会被吸附顶掉。
4. **滚动只有一个口径。** 行数 = `layout::content_rows(items, cols, |_, it| (it.grid_col,
   it.grid_row))`（**禁止**再用 `items.len()/cols`：空洞与手动放的深行会被算短，
   滚动条压根不出现），上限 = `layout::scroll_limit(rows, cell_h, visible_h)`，
   `visible_h` 一律以 `grid_top` 起算。滚轮（`WM_MOUSEWHEEL`）、`WM_SIZE` 收敛、
   `d2d_render` 画滚动条三处必须都走 `grid_layout::scroll_metrics`，否则会出现
   「滚得动却没滚动条 / 有滚动条滚不到底」。滚轮按**行**滚（`SCROLL_ROWS_PER_NOTCH`），
   不按裸像素，否则 192/96 DPI 下每格距离差一倍。

**归属**：这一组是宿主内容层的排布与滚动，`liquid-verre-rs` 只提供玻璃块与把手；
滚轮事件由库的 sink 转成 `WM_MOUSEWHEEL` 交给宿主（`glass_engine::PointerPhase::Wheel`），
**不需要改库**。

验证用例（已入库）：`cols_are_dpi_invariant_across_monitors`、
`snap_width_only_absorbs_rounding_noise`、`relayout_for_size_wraps_fixed_items_into_narrower_box`、
`relayout_for_dpi_keeps_cols_across_monitors`、`scroll_metrics_counts_deep_fixed_rows`、
`content_rows_counts_fixed_positions_beyond_linear_flow`。
### 借用纪律：绝不在持有 `&mut GridWindowData` 时调用同步 Win32

`SetWindowPos` / `EndDeferWindowPos` 会在**本线程同步重入**窗口过程，嵌套的消息处理会
再次 `get_data(hwnd)` 取一个 `&mut GridWindowData` —— 两个可变引用指向同一块内存即 UB。
`glass_session_active` 一类的上游守卫只是「碰巧挡住了重入分支」，任何一处漏判就破口。

- 固定写法：**读 → 结束借用 → 做动作 → 需要时重新 `get_data` 写回**；
- 需要跨动作使用的值（`config_id` / `render_padding` / `dpi` / 矩形）一律先抄成局部变量；
- 本仓库自持的 `RECT` 既非 `Copy` 也非 `Clone`，复用请显式 `copy_rect()`，
  不要为两个字段去改跨模块共用的 FFI 类型派生。

已按此纪律改造：`WM_WINDOWPOSCHANGED` 的吸附分支、`grid_window::follow_glass_session`、
`grid_window::end_glass_resize_session`。

### 会话期保护：任何回推库的入口都要判 `block_in_session`

`WM_MOVING` 里给玻璃块回推几何同样要 `!glass_session_active && !block_in_session(...)`：
会话起止是 PostMessage 异步送达的，标志位还没置上的那几拍里反推一次，就会把库的会话锚点搞脏。

### 落盘与 DPI 基准（已知遗留，改动前先读）

- `collect_grid_layout` 把「物理坐标 ÷ 当前窗口 DPI」当逻辑坐标写盘，
  `create_grid_window` 恢复时又按「建窗初始落点的 DPI」缩放 ——
  **跨 DPI 屏恢复的缩放基准可能与目标屏不符**（历史上表现为副屏的框位置飘）。
  彻底修需要把「显示器身份 + 屏内相对坐标」写进配置；本阶段的兜底是
  「无论基准怎么偏，吸附都会把框放回某块屏内」，并补了双屏异 DPI 的回归测试要求。
- `collect_grid_layout` 可能在 Tauri 命令线程执行：这里**只规范化要写进配置的值**，
  不搬窗口（跨线程 `SetWindowPos` 会同步等待平台线程，历史前科是卡死 WebView2 事件循环）。

### 检查清单

- [ ] 新增的几何推进入口是否经过单屏吸附（或明确属于「库会话镜像」例外）？
- [ ] 吸附是否幂等、是否可能触发 `WM_WINDOWPOSCHANGED` 回环抖动？
- [ ] 是否在库会话期间反向 push 了块矩形？（禁止）
- [ ] 归属屏判据是否用块中心？是否与库 `rehome_block` 一致？
- [ ] 跨线程路径是否避免了对平台线程窗口的同步调用？
- [ ] 是否在持有 `&mut GridWindowData` 时调用了会重入消息的 Win32（SetWindowPos 等）？
- [ ] 回推玻璃块几何的入口是否都判了 `block_in_session`？
- [ ] 阈值/边界纯函数是否落在 `box_core` 并带单测？

## 开机自启（Registry Run）健壮性

### 写入规则

- Run 键值必须**用双引号包裹 exe 路径**：Run 值按命令行解析，含空格路径不加引号会启动失败。
- 开启自启时同时写 `HKCU\\...\\Explorer\\StartupApproved\\Run` 启用标记
  （REG_BINARY 12 字节，首字节 `0x02`）；任务管理器/启动影响评估/安全软件可能写入
  `0x03` 禁用标记，导致“设置了自启却不启动”。关闭自启时删除该标记。
- 每次启动都按配置重新同步 Run 值（自愈），保证 exe 路径迁移后注册表跟随更新。

### 自毁路径防护（历史教训，必读）

- **绝不能按回退默认值同步系统状态**：配置加载失败时 `config` 是默认值
  （`auto_start=false`），此时若照常执行 `set_auto_start(config.general.auto_start)`，
  会把用户已开启的自启条目**删掉**。因此 `AppContext` 提供 `config_loaded` 标志，
  配置未成功加载时必须跳过自启同步（见 `platform_runtime.rs::sync_auto_start`）。
- **配置必须原子写入**：`AppConfig::save()` 走 `atomic_write_file`（临时文件 +
  `rename`）。直接 `fs::write` 覆盖时，进程在写入中途被杀（强制关机/断电）会产生
  半截配置文件 → 下次启动配置解析失败 → 触发上面的回退路径 → 自启被删。这是
  “开机自启时灵时不灵”的最隐蔽根因。
- **自启同步必须放在启动流程最前面**：排在托盘初始化、布局恢复等耗时/可能失败的
  步骤之前，确保进程只要被拉起，注册表状态就会被重写（自愈抵御 0x03 禁用）。
- **同步失败要重试并记日志**：`sync_auto_start` 最多重试 3 次，每次失败记
  `log::warn!`，连续失败记 `log::error!`。

### 排查手段

- `main.rs::log_boot_context` 启动时记录系统运行时长与 exe 路径；
  时长 < 180s 提示“可能由开机自启触发”。
- `platform.rs::diagnose_auto_start_state` 启动时记录 Run 键原始值（是否指向当前
  exe）与 StartupApproved 首字节（02=启用 / 03=禁用），区分三种失效：
  无启动日志 = Windows 没拉起；条目指向旧路径 = 路径漂移；03 = 被禁用。
- `main.rs::install_panic_hook` 把 panic 与回溯写入日志，区分“没拉起”与“拉起后崩溃”。
- 开机后检查 `~/.ikun_organizer_log/`：有启动日志 → Windows 已拉起（问题在程序内）；
  无日志 → Windows 未拉起（检查 Run 值路径是否存在、StartupApproved 标记、安全软件）。
- **日志级别默认必须是 info**：`init_logger` 使用
  `Env::default().default_filter_or("info")`。若默认 error，上述诊断日志在用户
  机器上不可见，排查手段全部失效（08-31 任务的根因之一）。

### 测试纪律

- **禁止让单元测试污染真实 Run 键**：操作注册表的测试必须在结束时把
  Run 值恢复到测试前的原始字节（见 `utils/platform.rs` 测试的
  `read_run_value` / `restore_run_value`）。历史教训：测试把临时
  `cargo test` 二进制路径写入 Run 键，重建后路径失效，造成开机自启“时灵时不灵”。
- 操作真实注册表的测试必须用静态锁串行化，避免并行竞争。
