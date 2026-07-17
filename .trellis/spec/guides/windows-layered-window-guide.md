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
