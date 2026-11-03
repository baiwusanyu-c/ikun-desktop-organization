import io

# ===== 问题4修复：apply_settings 里也要夹取最小尺寸 =====
p = r"D:\chapanda-project\chapanda-ds-organizers\src\platform\windows\glass_engine.rs"
s = io.open(p, encoding="utf-8").read()

old = """    holder.style_cfg = *glass;
    let style = glass_style_from(glass);
    let resize = resize_options_from(&glass.resize);
    holder.engine.set_default_style(style);
    holder.engine.set_default_resize(resize);
    for entry in holder.boxes.values_mut() {
        entry.window.set_style(Some(style));
        if !entry.locked {
            entry.window.set_resize_options(resize);
        }
    }"""
new = """    holder.style_cfg = *glass;
    let style = glass_style_from(glass);
    let mut resize = resize_options_from(&glass.resize);
    // 与 block_config_for 同口径夹取：否则样式切换会把原始配置的 605x1150
    // 直接推给每个块 set_resize_options，框又被钉死（2026-11-03 用户复验）。
    let primary = get_primary_monitor_dpi();
    for entry in holder.boxes.values_mut() {
        let dpi = (entry.ratio * primary as f32).round() as u32;
        let (fw, fh) = glass_resize_floor(dpi);
        let mut r = resize;
        if r.min_width > fw || r.min_height > fh {
            r.min_width = r.min_width.min(fw);
            r.min_height = r.min_height.min(fh);
        }
        entry.window.set_style(Some(style));
        if !entry.locked {
            entry.window.set_resize_options(r);
        }
    }
    // default_resize 也要夹（新建块的兜底）。
    let primary_dpi = primary;
    let (dfw, dfh) = glass_resize_floor(primary_dpi);
    if resize.min_width > dfw || resize.min_height > dfh {
        resize.min_width = resize.min_width.min(dfw);
        resize.min_height = resize.min_height.min(dfh);
    }"""
assert old in s
s = s.replace(old, new, 1)
# set_default_resize 也要放在夹取之后
s = s.replace("""    holder.engine.set_default_style(style);
    holder.engine.set_default_resize(resize);
    for entry in holder.boxes.values_mut() {""", """    holder.engine.set_default_style(style);
    holder.engine.set_default_resize(resize);
    for entry in holder.boxes.values_mut() {""", 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
print("p4 patched")

# ===== 问题3修复：guide overlay 加 300ms 宽限期 + 检测矩形改回窗口矩形 =====
p2 = r"D:\chapanda-project\chapanda-ds-organizers\src\platform\windows\alignment_guide\guide_overlay.rs"
s2 = io.open(p2, encoding="utf-8").read()
old2 = """    // 本轮没有任何对齐命中 -> 直接隐藏
    let has_lines =
        !alignment.horizontal_matches.is_empty() || !alignment.vertical_matches.is_empty();

    if !has_lines {
        // 隐藏窗口，保留实例下次复用
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        return;
    }"""
new2 = """    // 本轮没有任何对齐命中 -> 保留上一帧画面 300ms 再隐藏。
    // 检测每 16ms 跑一次，但移动框矩形有 1~2 拍延迟，命中是间歇性的：
    // 立刻隐藏 = 参考线闪一帧就消失，肉眼看不到（2026-11-03 用户报「不显示」）。
    let has_lines =
        !alignment.horizontal_matches.is_empty() || !alignment.vertical_matches.is_empty();

    if !has_lines {
        use std::time::Instant;
        static LAST_HIT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        let now = Instant::now();
        let should_hide = match LAST_HIT.lock() {
            Ok(g) => g.map(|t| now.duration_since(t).as_millis() >= 300).unwrap_or(true),
            Err(_) => true,
        };
        if should_hide {
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
        // should_hide=false 时直接 return：不调 UpdateLayeredWindow，
        // 分层窗保留上一帧位图 = 参考线继续可见。
        return;
    }
    // 有命中：刷新宽限期时间戳
    {
        use std::time::Instant;
        static LAST_HIT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        if let Ok(mut g) = LAST_HIT.lock() {
            *g = Some(Instant::now());
        }
    }"""
assert old2 in s2
s2 = s2.replace(old2, new2, 1)
io.open(p2, "w", encoding="utf-8", newline="\n").write(s2)
print("p3 grace patched")

# 问题3 修复b：alignment_rect 改回窗口矩形（库快照在会话期间可能延迟）
p3 = r"D:\chapanda-project\chapanda-ds-organizers\src\platform\windows\alignment_guide\mod.rs"
s3 = io.open(p3, encoding="utf-8").read()
old3 = """fn alignment_rect(hwnd: *mut std::ffi::c_void) -> Option<RECT> {
    if super::glass_engine::is_active() {
        let box_id = unsafe { grid_window::get_data(hwnd) }.map(|d| d.config_id);
        if let Some(id) = box_id.filter(|id| super::glass_engine::has_box(*id)) {
            if let Some(rect) = super::glass_engine::block_content_rect(id) {
                return Some(rect);
            }
        }
    }
    grid_window::get_grid_content_rect(hwnd)
}"""
new3 = """fn alignment_rect(hwnd: *mut std::ffi::c_void) -> Option<RECT> {
    // 用代理窗口的内容矩形（物理像素），不用库块快照：会话期间库快照可能
    // 落后 1~2 拍（snapshot_dirty 不在几何变化时置位），拿快照检测 = 命中间歇
    // 性 → 参考线闪一帧就消失（2026-11-03 实测日志：8 次拖拽只命中 1 次）。
    // 窗口矩形由 follow_glass_session 每 tick 跟随块几何，响应更及时。
    grid_window::get_grid_content_rect(hwnd)
}"""
assert old3 in s3
s3 = s3.replace(old3, new3, 1)
io.open(p3, "w", encoding="utf-8", newline="\n").write(s3)
print("p3 rect patched")
