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

## Related Files

- `src/platform/windows/drag_handler.rs` - Drag state management
- `src/platform/windows/drag_image.rs` - Drag ghost window
- `src/platform/windows/desktop_overlay/overlay_wnd_proc.rs` - Overlay drag handling
- `src/platform/windows/desktop_overlay/overlay_window.rs` - Overlay state management
