# Windows Layered Window Guide (WS_EX_LAYERED)

> Understanding pixel-level alpha behavior for mouse event routing

---

## Overview

This guide documents the **implicit behavior** of WS_EX_LAYERED windows that caused 7 failed debug attempts before finding the correct solution. The key insight: **Windows performs pixel-level alpha testing BEFORE WM_NCHITTEST**, making traditional hit-testing approaches ineffective for alpha=0 pixels.

---

## Core Behavior Model

### Message Routing Order

`
Mouse Event → Pixel Alpha Test → WM_NCHITTEST → Window Receives Event
                ↓ (alpha=0)
            Event Passes Through (transparent to window)
            WM_MOUSELEAVE triggered repeatedly
`

### Alpha Value Effects

| Alpha | Behavior | Use Case |
|-------|----------|----------|
| 0 | **Pixel doesn't exist** - events pass through, WM_MOUSELEAVE fires | Click-through to desktop/other apps |
| 1-254 | Pixel exists but semi-transparent - receives all events | Hover states, subtle highlights |
| 255 | Fully opaque - receives all events | Solid UI elements |

### Critical Rules

1. **WM_NCHITTEST is bypassed for alpha=0 pixels**
   - Returning HTCLIENT does NOT prevent pass-through
   - Returning HTTRANSPARENT is redundant (already transparent)

2. **WM_MOUSELEAVE fires continuously on alpha=0 pixels**
   - System considers mouse as "leaving" the window
   - Causes hover state flickering if not handled correctly
   - **Solution**: Ignore WM_MOUSELEAVE, let WM_MOUSEMOVE manage hover naturally

3. **Alpha testing happens at pixel level**
   - Each pixel's alpha is tested independently
   - A window can have mixed transparent/opaque regions
   - Mouse events only reach opaque pixels (alpha > 0)

---

## Common Pitfalls

### Pitfall 1: Trying to use WM_NCHITTEST for click-through

`ust
// ❌ WRONG: This doesn't work for alpha=0 pixels
WM_NCHITTEST => {
    if is_blank_area {
        HTTRANSPARENT  // System already bypassed this!
    } else {
        HTCLIENT
    }
}
`

**Why it fails**: Alpha=0 pixels never reach WM_NCHITTEST - they pass through at the system level.

**Correct approach**: Control transparency via background alpha:
`ust
// ✅ CORRECT: Dynamic alpha based on state
let bg_alpha = if has_selection { 1 } else { 0 };
target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
`

### Pitfall 2: Relying on WM_MOUSELEAVE for hover management

`ust
// ❌ WRONG: Causes flickering on alpha=0 pixels
WM_MOUSELEAVE => {
    data.hovered_item = -1;  // Keeps getting reset!
    render();
}
`

**Why it fails**: WM_MOUSELEAVE fires repeatedly on transparent pixels, causing hover state to flicker.

**Correct approach**: Ignore WM_MOUSELEAVE, use WM_MOUSEMOVE hit test:
`ust
// ✅ CORRECT: Let WM_MOUSEMOVE manage hover naturally
WM_MOUSELEAVE => {
    // Do nothing - WS_EX_LAYERED triggers this unreliably
    0
}

WM_MOUSEMOVE => {
    let hit = hit_test(x, y);
    if data.hovered_item != hit {
        data.hovered_item = hit;
        render();
    }
}
`

### Pitfall 3: Forwarding events via PostMessageW

`ust
// ❌ WRONG: Complex, unreliable, breaks right-click menus
if is_blank_area {
    let desktop_hwnd = FindWindowW(...);
    PostMessageW(desktop_hwnd, WM_LBUTTONDOWN, ...);
}
`

**Why it fails**: 
- Window hierarchy discovery is fragile (Progman vs WorkerW)
- Coordinate conversion is error-prone
- Right-click menus break (menu needs to know which window owns it)
- Fighting against platform behavior instead of working with it

**Correct approach**: Use alpha=0 for natural pass-through:
`ust
// ✅ CORRECT: Let the system handle pass-through
let bg_alpha = if has_selection { 1 } else { 0 };
target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
// When alpha=0, clicks naturally pass through to desktop
`

---

## Design Pattern: State-Driven Transparency

### Problem
Need click-through when idle, but need to capture clicks when in interactive state (e.g., has selection).

### Solution: Dynamic Alpha

`ust
fn render() {
    // Determine if we need to capture clicks
    let need_click_capture = data.selected_item >= 0;
    
    // Set background alpha accordingly
    let bg_alpha: u8 = if need_click_capture { 1 } else { 0 };
    target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
    
    // Render content (icons, etc.)
    draw_content();
}
`

### State Machine

`
┌─────────────────┐
│  No Selection   │ ← alpha=0 (click-through)
│  (Idle State)   │
└────────┬────────┘
         │ click icon
         ↓
┌─────────────────┐
│  Has Selection  │ ← alpha=1 (capture clicks)
│  (Interactive)  │
└────────┬────────┘
         │ click blank area
         ↓
    deselect → back to idle
`

### Benefits

1. **Works WITH platform behavior** - not against it
2. **No event forwarding needed** - system handles pass-through
3. **Right-click menus work correctly** - no PostMessageW hacks
4. **Simple and reliable** - one alpha value controls everything

---

## Debug Checklist

When debugging layered window issues:

- [ ] **Check background alpha** - Is it 0 or >0? This determines event routing.
- [ ] **Monitor WM_MOUSELEAVE** - Is it firing too often? If yes, ignore it.
- [ ] **Verify WM_MOUSEMOVE coordinates** - Are they screen or client coordinates?
- [ ] **Test WM_NCHITTEST** - Does it even get called? (alpha=0 pixels bypass it)
- [ ] **Check for PostMessageW** - Are you forwarding events? This is usually wrong.
- [ ] **Test right-click menus** - Do they work? If not, you're probably forwarding events.

---

## Debug Logging Template

Add these logs to diagnose layered window issues:

`ust
// In WM_MOUSELEAVE handler
WM_MOUSELEAVE => {
    log::debug!("[MOUSELEAVE] fired at {:?}", std::time::Instant::now());
    // If this fires repeatedly, you need to ignore it
}

// In WM_MOUSEMOVE handler
WM_MOUSEMOVE => {
    let (x, y) = get_mouse_pos();
    let hit = hit_test(x, y);
    log::debug!("[MOUSEMOVE] ({}, {}), hit={}, hovered={}, selected={}", 
                x, y, hit, data.hovered_item, data.selected_item);
}

// In WM_LBUTTONDOWN handler
WM_LBUTTONDOWN => {
    let (x, y) = get_mouse_pos();
    let hit = hit_test(x, y);
    log::info!("[CLICK] ({}, {}), hit={}, selected={}", 
               x, y, hit, data.selected_item);
}

// In render function
fn render() {
    let bg_alpha = if data.selected_item >= 0 { 1 } else { 0 };
    log::debug!("[RENDER] bg_alpha={}, selected={}", bg_alpha, data.selected_item);
    target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
}
`

---

## Cross-Platform Comparison

### Windows (WS_EX_LAYERED)
- Pixel-level alpha testing
- Alpha=0 → pass-through + WM_MOUSELEAVE
- Solution: Dynamic background alpha

### macOS (NSWindow)
- ignoresMouseEvents property (boolean)
- No pixel-level testing
- Solution: Toggle ignoresMouseEvents based on state

### Linux (X11/Wayland)
- Shape extension for non-rectangular windows
- Input region can be set explicitly
- Solution: XShapeCombineRegion or Wayland input region

---

## Related Files

- src/platform/windows/desktop_overlay/overlay_render.rs - Background alpha logic
- src/platform/windows/desktop_overlay/overlay_wnd_proc.rs - Message handling
- src/platform/windows/d2d_render.rs - D2D rendering utilities

---

## References

- [Layered Windows (Microsoft Docs)](https://docs.microsoft.com/en-us/windows/win32/winmsg/window-features#layered-windows)
- [UpdateLayeredWindow function](https://docs.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow)
- [WS_EX_LAYERED extended window style](https://docs.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles)

---

## History

This guide was created after **7 failed debug attempts** (commits de9ed40 through 72c8f73) to fix hover flickering and click-to-deselect bugs. The root cause was an implicit assumption about WM_NCHITTEST behavior that didn't account for pixel-level alpha testing.

**Key lesson**: When working with platform-specific features, always test the actual behavior rather than relying on documentation or assumptions. The platform's implicit behaviors can completely invalidate your mental model.

---

## Drag and Drop on Layered Windows

> Lessons learned from fixing 7 drag-related bugs in 2 days

### SetCapture Limitations

**Critical Rule**: `SetCapture` is **unstable** on WS_EX_LAYERED windows.

```rust
// WRONG: Causes infinite WM_CAPTURECHANGED loop
SetCapture(layered_hwnd);
// -> Windows immediately sends WM_CAPTURECHANGED
// -> If you re-capture in handler -> infinite loop
```

**Why it fails**:
- WS_EX_LAYERED windows have special message routing
- SetCapture conflicts with layered window's alpha-based hit testing
- Windows forcibly releases capture, triggering WM_CAPTURECHANGED

**Correct approach**: Use Timer-based polling

```rust
// CORRECT: Timer polling for drag state
const DRAG_TIMER_ID: usize = 0x4F56_0001;

fn start_drag(hwnd: HWND) {
    data.drag_active = true;
    SetTimer(hwnd, DRAG_TIMER_ID, 8, null_mut()); // 8ms = 125Hz
}

fn handle_timer(hwnd: HWND, timer_id: usize) {
    if timer_id == DRAG_TIMER_ID {
        if !data.drag_active {
            // State was reset externally, cleanup
            cleanup_drag(hwnd);
            return;
        }
        
        // Update drag ghost position
        update_drag_ghost();
        
        // Check if mouse button still pressed
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

### Message Routing on Layered Windows

**Key insight**: Layered windows in bottom Z-order **do not receive mouse messages** unless they have capture.

```
+------------------+
|  Other Windows   |  <- Mouse messages go here (top Z-order)
+------------------+
| Layered Window   |  <- No mouse messages (bottom Z-order)
| (alpha=0 area)   |     Unless: SetCapture (unstable) or Timer polling
+------------------+
```

**Implications**:
- Cannot rely on `WM_MOUSEMOVE` for drag tracking
- Cannot rely on `WM_LBUTTONUP` to end drag
- Must use Timer to poll mouse state

### Drag Ghost Positioning

**Common pitfall**: Assuming window position = icon position

```rust
// WRONG: Window position != icon position
SetWindowPos(ghost_hwnd, null_mut(), cursor_x, cursor_y, ...);
// Icon is drawn centered in window, so actual icon position is offset
```

**Correct approach**: Account for icon centering offset

```rust
// CORRECT: Subtract centering offset
let icon_offset_x = (window_width - icon_size) / 2;
SetWindowPos(ghost_hwnd, null_mut(), 
             cursor_x - icon_offset_x, cursor_y, ...);
```

### State Management Anti-Patterns

**Problem**: Multiple modules can modify `drag_active`, causing state inconsistency

```rust
// WRONG: Direct field access from multiple modules
// In drag_handler.rs:
data.drag_active = true;

// In desktop_watcher.rs:
data.drag_active = false; // Reset during refresh!
// -> Drag ghost stuck because Timer sees drag_active=false
```

**Correct approach**: Centralized state manager with validation

```rust
// CORRECT: Validate state consistency
pub fn handle_timer() {
    if !data.drag_active {
        // State was reset externally, force cleanup
        log::warn!("[DRAG] State reset externally, cleaning up");
        cleanup_ghost();
        KillTimer(hwnd, DRAG_TIMER_ID);
        return;
    }
    
    if !is_mouse_pressed() {
        end_drag();
    }
}
```

### Debug Checklist for Drag Issues

- [ ] **Timer vs SetCapture**: Using Timer on layered windows? (SetCapture is unstable)
- [ ] **State consistency**: Is `drag_active` synchronized with Timer state?
- [ ] **External state modification**: Can other modules reset drag state?
- [ ] **Ghost positioning**: Accounted for icon centering offset?
- [ ] **Z-order**: Is layered window receiving messages? (Check Z-order)
- [ ] **Cleanup on error**: Does Timer cleanup ghost if state is inconsistent?

### Common Timer Issues

**Issue 1: Timer too slow (lag)**

```rust
// WRONG: 16ms timer + expensive DPI check = lag
SetTimer(hwnd, DRAG_TIMER_ID, 16, null_mut());
fn handle_timer() {
    check_dpi(); // Expensive system call every frame!
    update_ghost();
}
```

```rust
// CORRECT: 8ms timer + cached DPI
SetTimer(hwnd, DRAG_TIMER_ID, 8, null_mut()); // 125Hz
static mut LAST_DPI: u32 = 96;
fn handle_timer() {
    // Only check DPI every 8 frames
    if frame_count % 8 == 0 {
        LAST_DPI = check_dpi();
    }
    update_ghost();
}
```

**Issue 2: Timer not cleaning up on state reset**

```rust
// WRONG: Timer only ends drag when mouse released
fn handle_timer() {
    if !is_mouse_pressed() {
        end_drag();
    }
    // What if drag_active was reset externally? Timer keeps running!
}
```

```rust
// CORRECT: Timer checks state consistency
fn handle_timer() {
    if !data.drag_active {
        // State was reset externally, force cleanup
        log::warn!("[DRAG] State reset externally, cleaning up");
        cleanup_ghost();
        KillTimer(hwnd, DRAG_TIMER_ID);
        return;
    }
    
    if !is_mouse_pressed() {
        end_drag();
    }
}
```
