import io
p = r"D:\chapanda-project\chapanda-ds-organizers\src\platform\windows\glass_engine.rs"
s = io.open(p, encoding="utf-8").read()
s = s.replace("resize.min_height = resize.min_height.min(fh);", "resize.min_height = resize.min_height.min(dfh);", 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
print("typo fixed")
