import io
p = r"D:\chapanda-project\chapanda-ds-organizers\src\platform\windows\glass_engine.rs"
s = io.open(p, encoding="utf-8").read()
old = """    let primary = get_primary_monitor_dpi();
    for entry in holder.boxes.values_mut() {"""
new = """    holder.engine.set_default_style(style);
    let primary = get_primary_monitor_dpi();
    for entry in holder.boxes.values_mut() {"""
assert old in s
s = s.replace(old, new, 1)
old2 = """    if resize.min_width > dfw || resize.min_height > dfh {
        resize.min_width = resize.min_width.min(dfw);
        resize.min_height = resize.min_height.min(dfh);
    }
    holder.engine.request_render();"""
new2 = """    if resize.min_width > dfw || resize.min_height > dfh {
        resize.min_width = resize.min_width.min(dfw);
        resize.min_height = resize.min_height.min(fh);
    }
    holder.engine.set_default_resize(resize);
    holder.engine.request_render();"""
assert old2 in s
s = s.replace(old2, new2, 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
print("fixed set_default_style/resize")
