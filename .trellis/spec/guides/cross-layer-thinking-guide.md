# Cross-Layer Thinking Guide

> How to think across UI, business, and platform layers to avoid cross-layer bugs

---

## Overview

This guide helps developers identify and prevent bugs that arise from **implicit assumptions** about how different layers interact. Most cross-layer bugs stem from undocumented platform behaviors or incorrect mental models about layer boundaries.

---

## The Three-Layer Model

`
┌─────────────────────────────────────┐
│         UI Layer                    │  ← User interactions, visual feedback
│  (GPUI views, dialogs, panels)      │
├─────────────────────────────────────┤
│       Business Layer                │  ← State management, data flow
│  (AppContext, AppConfig, services)  │
├─────────────────────────────────────┤
│      Platform Layer                 │  ← OS APIs, window management, rendering
│  (Windows/macOS/Linux specifics)    │
└─────────────────────────────────────┘
`

### Key Principle

**Each layer has implicit behaviors that are not documented in the layer above it.**

- Platform layer has OS-specific behaviors (e.g., WS_EX_LAYERED alpha testing)
- Business layer has state transition rules (e.g., when is a selection valid?)
- UI layer has interaction patterns (e.g., hover vs click vs drag)

When you assume behavior at one layer without understanding the layer below, bugs emerge.

---

## Common Cross-Layer Bug Patterns

### Pattern 1: Platform Implicit Behavior

**Symptom**: UI logic works in isolation but fails in real usage.

**Example**: Hover flickering on layered windows
- UI layer: WM_MOUSEMOVE updates hover state
- Business layer: hovered_item tracks which icon is hovered
- Platform layer: WS_EX_LAYERED with alpha=0 pixels bypasses WM_NCHITTEST and triggers WM_MOUSELEAVE

**Root cause**: UI layer assumed WM_NCHITTEST controls all mouse routing, but platform layer has pixel-level alpha testing that happens first.

**Prevention**: 
- [ ] Check platform-specific guides before implementing interaction logic
- [ ] Test actual behavior, don't rely on documentation
- [ ] See [Windows Layered Window Guide](./windows-layered-window-guide.md)

### Pattern 2: State-Platform Mismatch

**Symptom**: Feature works in some states but not others.

**Example**: Click-to-deselect on overlay
- UI layer: Click blank area should deselect icon
- Business layer: selected_item = -1 when clicking blank
- Platform layer: Alpha=0 pixels pass through clicks, so blank area clicks never reach window

**Root cause**: Business logic assumed all clicks reach the window, but platform layer filters based on pixel alpha.

**Prevention**:
- [ ] Map out which states need which platform behaviors
- [ ] Use state-driven platform features (e.g., dynamic alpha)
- [ ] Test all state transitions, not just happy path

### Pattern 3: Cross-Layer Event Forwarding

**Symptom**: Complex PostMessage / SendMessage chains that break in edge cases.

**Example**: Forwarding right-click to desktop
- UI layer: Need to show desktop context menu
- Business layer: Determine if click is on blank area
- Platform layer: Use FindWindowW + PostMessageW to forward event

**Root cause**: Fighting against platform behavior instead of working with it.

**Prevention**:
- [ ] Ask: "Am I forwarding events? If yes, is there a simpler way?"
- [ ] Prefer platform-native solutions (e.g., alpha=0 for pass-through)
- [ ] Event forwarding is a red flag - usually indicates wrong approach

---

## Debug Methodology

### Step 1: Identify the Layers Involved

Ask:
- Which UI component is broken?
- What business state is active?
- What platform API is being called?

### Step 2: Test Each Layer in Isolation

- **UI layer**: Does the logic work with mock data?
- **Business layer**: Does state transition correctly?
- **Platform layer**: Does the API behave as expected?

### Step 3: Check Implicit Assumptions

For each layer, ask:
- What am I assuming about this layer's behavior?
- Is this assumption documented?
- Have I tested this assumption?

### Step 4: Add Diagnostic Logging

Add logs at layer boundaries:

`ust
// Platform layer boundary
WM_MOUSEMOVE => {
    log::debug!("[PLATFORM] WM_MOUSEMOVE ({}, {})", x, y);
    // ... convert to business event
}

// Business layer boundary
fn update_hover(hit: i32) {
    log::debug!("[BUSINESS] update_hover: {} -> {}", data.hovered_item, hit);
    // ... update state
}

// UI layer boundary
fn render() {
    log::debug!("[UI] render: hovered={}, selected={}", data.hovered_item, data.selected_item);
    // ... draw
}
`

### Step 5: Trace the Bug

Follow the logs to see where the assumption breaks:
- If platform log shows unexpected behavior → platform implicit behavior
- If business log shows wrong state → state transition bug
- If UI log shows wrong render → rendering logic bug

---

## Prevention Checklist

Before implementing cross-layer features:

- [ ] **Read platform guides** - Check .trellis/spec/guides/ for platform-specific docs
- [ ] **Map state transitions** - Draw state machine for the feature
- [ ] **Test platform behavior** - Write minimal test case to verify assumptions
- [ ] **Avoid event forwarding** - Use platform-native solutions when possible
- [ ] **Add boundary logs** - Log at layer transitions for debugging
- [ ] **Test all states** - Don't just test happy path, test edge cases

---

## Anti-Patterns

### Anti-Pattern 1: "Just Forward the Event"

`ust
// ❌ BAD: Complex event forwarding
if is_blank_area {
    let desktop = FindWindowW(...);
    PostMessageW(desktop, WM_RBUTTONUP, ...);
}
`

**Why it's bad**: 
- Fragile (window hierarchy changes)
- Breaks context (menu doesn't know owner window)
- Hard to debug (events jump between windows)

**Better**: Use platform-native pass-through (alpha=0)

### Anti-Pattern 2: "The Documentation Says..."

`ust
// ❌ BAD: Trusting documentation without testing
WM_NCHITTEST => {
    // Docs say HTCLIENT receives all events
    HTCLIENT
}
`

**Why it's bad**: Documentation doesn't cover all implicit behaviors (e.g., alpha testing).

**Better**: Test actual behavior with minimal repro

### Anti-Pattern 3: "It Works in the Test"

`ust
// ❌ BAD: Unit test passes but integration fails
#[test]
fn test_hover_logic() {
    let mut data = OverlayData::new();
    data.update_hover(1);
    assert_eq!(data.hovered_item, 1);  // ✅ Passes
}

// But in real usage, WM_MOUSELEAVE keeps resetting it
`

**Why it's bad**: Unit test doesn't exercise platform layer behavior.

**Better**: Add integration test or manual test checklist

---

## Case Study: Overlay Click-to-Deselect

**Problem**: Clicking blank area on overlay doesn't deselect icon.

**Failed Attempts** (7 commits):
1. de9ed40 - Forward right-click via PostMessageW → broke menus
2. 12fd4d9 - Restore alpha=0 + HTTRANSPARENT → can't capture clicks
3. c520683 - Use WM_MOUSEACTIVATE → alpha=0 bypasses this too
4. 540d03d - Always return HTCLIENT → alpha=0 still bypasses
5. ebdbdb1 - Fix UTF-16 encoding → forwarding still doesn't work
6. b6731d - Fix coordinate conversion → still fighting platform behavior
7. 72c8f73 ✅ - Dynamic alpha based on selection state → **works with platform**

**Root Cause**: Implicit assumption that WM_NCHITTEST controls all mouse routing. Reality: WS_EX_LAYERED has pixel-level alpha testing that happens first.

**Solution**: 
`ust
let bg_alpha = if data.selected_item >= 0 { 1 } else { 0 };
target.Clear(Some(&d2d_color(0, 0, 0, bg_alpha)));
`

**Lesson**: Work with platform behavior, don't fight it.

---

## Related Guides

- [Windows Layered Window Guide](./windows-layered-window-guide.md) - Pixel-level alpha behavior
- [Windows Shell API Guide](./windows-shell-api-guide.md) - COM interfaces and menus
- [Code Reuse Thinking Guide](./code-reuse-thinking-guide.md) - Avoiding duplicate logic

---

## References

- [Layered Windows (Microsoft Docs)](https://docs.microsoft.com/en-us/windows/win32/winmsg/window-features#layered-windows)
- [Event Routing in Windows](https://docs.microsoft.com/en-us/windows/win32/inputdev/about-mouse-input)

---

## History

This guide was created after debugging the overlay hover flickering and click-to-deselect bugs, which required 7 failed attempts before identifying the root cause as a cross-layer implicit assumption about WS_EX_LAYERED behavior.

---

## Case Study: Drag State Cross-Layer Failure (2026-07-20)

### Problem

Desktop file watcher (background thread) resets overlay data during refresh, breaking drag state managed by UI thread.

**Timeline**:
```
UI Thread:              Background Thread:
drag_start() -> active=true
drag_update() -> active=true  
                        desktop_watcher detects file change
                        refresh_overlay() -> reset_overlay_data() -> active=false
drag_update() -> active=false (unexpected!)
drag_ghost stuck (Timer sees false, cleans up)
```

### Root Cause

```rust
// desktop_overlay/overlay_window.rs
fn refresh_desktop_overlay_icons(hwnd: HWND) {
    // ... collect new icons ...
    reset_overlay_interaction_state(data);  // <- Resets drag_active!
}

fn reset_overlay_interaction_state(data: &mut OverlayWindowData) {
    data.selected_item = -1;
    data.hovered_item = -1;
    data.drag_pending = false;
    data.drag_active = false;  // <- BUG: Breaks ongoing drag
    data.drag_item_index = -1;
}
```

### Solution

**Defensive check**: Skip refresh if drag is active

```rust
pub(crate) fn refresh_desktop_overlay_icons(hwnd: HWND) {
    // Skip refresh during drag to prevent state reset
    if let Some(data) = get_overlay_data(hwnd) {
        if data.drag_active || data.drag_pending {
            log::debug!("[OVERLAY] Skip refresh during drag");
            return;
        }
    }
    // ... normal refresh ...
}
```

**Fail-safe cleanup**: Timer detects inconsistent state

```rust
fn handle_overlay_drag_timer(hwnd: HWND) {
    let is_dragging = match get_overlay_data(hwnd) {
        Some(data) => data.drag_active,
        None => return,
    };
    
    if !is_dragging {
        // State was reset externally, force cleanup
        log::warn!("[DRAG] Timer detected state reset, cleaning up");
        unsafe {
            KillTimer(hwnd, TIMER_ID_OVERLAY_DRAG);
            crate::platform::windows::drag_image::destroy_drag_overlay();
        }
        return;
    }
    // ... normal drag update ...
}
```

### Lessons Learned

1. **Cross-thread state modification is dangerous**: Background threads should not directly modify UI state
2. **Defensive checks are essential**: Always validate state before operations
3. **Fail-safe cleanup prevents stuck states**: Timers should detect and recover from inconsistent states
4. **Logging is critical**: Without DIAG logs, this bug would have been impossible to diagnose

### Prevention Checklist

- [ ] **Thread safety**: Does background thread modify UI state?
- [ ] **State validation**: Do operations check state before executing?
- [ ] **Fail-safe cleanup**: Can system recover from inconsistent states?
- [ ] **Cross-module dependencies**: Which modules can reset shared state?
- [ ] **Logging coverage**: Are state transitions logged for debugging?
