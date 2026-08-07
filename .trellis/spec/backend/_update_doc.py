import re
import sys

filepath = r"D:\github-project\ikun_organizer\.trellis\spec\backend\directory-structure.md"

with open(filepath, 'r', encoding='utf-8') as f:
    content = f.read()

changes = 0

# 1. Update context.rs description
old1 = "\u251c\u2500\u2500 context.rs                 # AppContext \u6838\u5fc3\u4e0a\u4e0b\u6587\u5bf9\u8c61"
new1 = "\u251c\u2500\u2500 context.rs                 # AppContext\uff08\u6301\u6709 Box<dyn DesktopManagerTrait> + Box<dyn PlatformService>\uff09"
if old1 in content:
    content = content.replace(old1, new1)
    changes += 1
    print("1. context.rs updated")
else:
    print("1. SKIP: context.rs pattern not found")

# 2. Insert shared/ block
old2 = new1 + "\n\u2502\n\u251c\u2500\u2500 gpui_app/"
new2 = new1 + """
\u2502
\u251c\u2500\u2500 shared/                    # \U0001f195 \u8de8\u5e73\u53f0\u5171\u4eab\u903b\u8f91\u5c42\uff08\u4ece Windows \u63d0\u53d6\u7684\u5e73\u53f0\u65e0\u5173\u4ee3\u7801\uff09
\u2502   \u251c\u2500\u2500 mod.rs                 #   \u6a21\u5757\u5bfc\u51fa
\u2502   \u251c\u2500\u2500 selection.rs           #   \u5168\u5c40\u591a\u9009\u6ce8\u518c\u8868\uff08\u8de8\u5bb9\u5668\u9009\u62e9/\u951a\u70b9/\u6846\u9009\uff09~526\u884c
\u2502   \u251c\u2500\u2500 drag_logic.rs          #   \u62d6\u62fd\u7eaf\u903b\u8f91\uff08compute_drag_over_state/apply_shift_insert\uff09~185\u884c
\u2502   \u251c\u2500\u2500 geometry.rs            #   \u51e0\u4f55\u8ba1\u7b97\uff08\u5bf9\u9f50\u68c0\u6d4b/\u50cf\u7d20\u8fb9\u754c/\u7b49\u6bd4\u5c45\u4e2d\uff09~325\u884c
\u2502   \u251c\u2500\u2500 layout.rs              #   \u5e03\u5c40\u91cd\u6392\u903b\u8f91\uff08reflow_positions\uff09~96\u884c
\u2502   \u251c\u2500\u2500 drag_state.rs          #   \u62d6\u62fd\u751f\u547d\u5468\u671f\u72b6\u6001\u673a+\u5e38\u91cf ~32\u884c
\u2502   \u2514\u2500\u2500 types.rs               #   \u8de8\u5e73\u53f0\u5171\u4eab\u7c7b\u578b\u5b9a\u4e49\uff08Rect/Point/Pixel32\uff09~52\u884c
\u2502
\u251c\u2500\u2500 gpui_app/"""
if old2 in content:
    content = content.replace(old2, new2)
    changes += 1
    print("2. shared/ block inserted")
else:
    print("2. SKIP: shared/ insertion pattern not found")
    # Debug: print chars around context.rs line
    idx = content.find("context.rs")
    if idx >= 0:
        snippet = content[idx-20:idx+80]
        print(f"   context.rs region: {repr(snippet)}")

# 3. Update platform/mod.rs
old3 = "\u2502   \u251c\u2500\u2500 mod.rs                 #   DesktopManagerTrait + re-exports"
new3 = "\u2502   \u251c\u2500\u2500 mod.rs                 #   DesktopManagerTrait(7\u65b9\u6cd5) + PlatformService(10\u65b9\u6cd5) + \u5de5\u5382\u51fd\u6570 + re-exports"
if old3 in content:
    content = content.replace(old3, new3)
    changes += 1
    print("3. platform/mod.rs updated")
else:
    print("3. SKIP: platform/mod.rs pattern not found")

# 4. Update platform/types.rs
old4 = "\u2502   \u251c\u2500\u2500 types.rs               #   \u8de8\u5e73\u53f0\u7c7b\u578b\uff1aDesktopIcon, IconInfo, ScreenInfo\n"
new4 = "\u2502   \u251c\u2500\u2500 types.rs               #   \u8de8\u5e73\u53f0\u7c7b\u578b\uff1aDesktopIcon, IconInfo, ScreenInfo, IconData\n"
if old4 in content:
    content = content.replace(old4, new4)
    changes += 1
    print("4. platform/types.rs updated")
else:
    print("4. SKIP: platform/types.rs pattern not found")

# 5a. grid_layout.rs in main tree
old5a = "\u2502   \u2502   \u251c\u2500\u2500 grid_layout.rs     #   \u5e03\u5c40\u8ba1\u7b97/\u6309\u94ae\u77e9\u5f62/\u547d\u4e2d\u6d4b\u8bd5/\u6eda\u52a8\u91cd\u6392"
new5a = "\u2502   \u2502   \u251c\u2500\u2500 grid_layout.rs     #   \u5e03\u5c40\u8ba1\u7b97/\u6309\u94ae\u77e9\u5f62/\u547d\u4e2d\u6d4b\u8bd5\uff08reflow_items \u59d4\u6258\u5230 shared/layout.rs\uff09"
if old5a in content:
    content = content.replace(old5a, new5a)
    changes += 1
    print("5a. grid_layout.rs (tree) updated")
else:
    print("5a. SKIP: grid_layout.rs (tree) pattern not found")

# 5b. multi_select.rs in main tree
old5b = "\u2502   \u2502   \u251c\u2500\u2500 multi_select.rs    #   \u5168\u5c40\u591a\u9009\u72b6\u6001\u7ba1\u7406\uff08\u8de8\u5bb9\u5668\u9009\u62e9/\u951a\u70b9/\u6846\u9009\uff09"
new5b = "\u2502   \u2502   \u251c\u2500\u2500 multi_select.rs    #   \u5168\u5c40\u591a\u9009 re-export\uff08\u5b9e\u9645\u5b9e\u73b0\u5728 shared/selection.rs\uff09"
if old5b in content:
    content = content.replace(old5b, new5b)
    changes += 1
    print("5b. multi_select.rs (tree) updated")
else:
    print("5b. SKIP: multi_select.rs (tree) pattern not found")

# 5c. drag_visual.rs in main tree
old5c = "\u2502   \u2502   \u251c\u2500\u2500 drag_visual.rs     #   \u62d6\u62fd\u53ef\u89c6\u5316\u53cd\u9988\uff08\u63d2\u5165\u5206\u9694\u7ebf/\u66ff\u6362\u9ad8\u4eae + \u7eaf\u903b\u8f91\u51fd\u6570\uff09"
new5c = "\u2502   \u2502   \u251c\u2500\u2500 drag_visual.rs     #   \u62d6\u62fd\u53ef\u89c6\u5316\u53cd\u9988\uff08HWND \u611f\u77e5\u90e8\u5206\uff0c\u7eaf\u903b\u8f91\u5df2\u63d0\u53d6\u5230 shared/drag_logic.rs\uff09"
if old5c in content:
    content = content.replace(old5c, new5c)
    changes += 1
    print("5c. drag_visual.rs (tree) updated")
else:
    print("5c. SKIP: drag_visual.rs (tree) pattern not found")

# 5d. grid_layout.rs in flat listing
old5d = "\u251c\u2500\u2500 grid_layout.rs     # \u5e03\u5c40\u8ba1\u7b97/\u6309\u94ae\u77e9\u5f62/\u547d\u4e2d\u6d4b\u8bd5/\u6eda\u52a8\u91cd\u6392"
new5d = "\u251c\u2500\u2500 grid_layout.rs     # \u5e03\u5c40\u8ba1\u7b97/\u6309\u94ae\u77e9\u5f62/\u547d\u4e2d\u6d4b\u8bd5\uff08reflow_items \u59d4\u6258\u5230 shared/layout.rs\uff09"
if old5d in content:
    content = content.replace(old5d, new5d)
    changes += 1
    print("5d. grid_layout.rs (flat) updated")
else:
    print("5d. SKIP: grid_layout.rs (flat) pattern not found")

# 5e. multi_select.rs in flat listing
old5e = "\u251c\u2500\u2500 multi_select.rs    # \u5168\u5c40\u591a\u9009\u72b6\u6001\u7ba1\u7406\uff08\u8de8\u5bb9\u5668\u9009\u62e9/\u951a\u70b9/\u6846\u9009\uff09"
new5e = "\u251c\u2500\u2500 multi_select.rs    # \u5168\u5c40\u591a\u9009 re-export\uff08\u5b9e\u9645\u5b9e\u73b0\u5728 shared/selection.rs\uff09"
if old5e in content:
    content = content.replace(old5e, new5e)
    changes += 1
    print("5e. multi_select.rs (flat) updated")
else:
    print("5e. SKIP: multi_select.rs (flat) pattern not found")

# 5f. drag_visual.rs in flat listing
old5f = "\u251c\u2500\u2500 drag_visual.rs     # \u62d6\u62fd\u53ef\u89c6\u5316\u53cd\u9988\uff08\u63d2\u5165\u5206\u9694\u7ebf/\u66ff\u6362\u9ad8\u4eae + \u7eaf\u903b\u8f91\u51fd\u6570\uff09"
new5f = "\u251c\u2500\u2500 drag_visual.rs     # \u62d6\u62fd\u53ef\u89c6\u5316\u53cd\u9988\uff08HWND \u611f\u77e5\u90e8\u5206\uff0c\u7eaf\u903b\u8f91\u5df2\u63d0\u53d6\u5230 shared/drag_logic.rs\uff09"
if old5f in content:
    content = content.replace(old5f, new5f)
    changes += 1
    print("5f. drag_visual.rs (flat) updated")
else:
    print("5f. SKIP: drag_visual.rs (flat) pattern not found")

# 6a. Add shared/ to allowed deps
old6a = "utils/ \u2192 (\u65e0\u5185\u90e8\u4f9d\u8d56)"
new6a = "shared/ \u2192 (\u65e0\u5185\u90e8\u4f9d\u8d56\uff0c\u4ec5 std + serde + uuid + lazy_static)\nplatform/ \u2192 shared (\u5141\u8bb8\uff1a\u5e73\u53f0\u5b9e\u73b0\u590d\u7528\u5171\u4eab\u903b\u8f91)\nutils/ \u2192 (\u65e0\u5185\u90e8\u4f9d\u8d56)"
if old6a in content:
    content = content.replace(old6a, new6a)
    changes += 1
    print("6a. allowed deps updated")
else:
    print("6a. SKIP: allowed deps pattern not found")

# 6b. Add shared/ to forbidden deps
old6b = "\u274c classification/ \u2192 ui/   (\u5206\u7c7b\u5c42\u4e0d\u4f9d\u8d56 UI)\n```"
new6b = "\u274c classification/ \u2192 ui/   (\u5206\u7c7b\u5c42\u4e0d\u4f9d\u8d56 UI)\n\u274c shared/ \u2192 platform/    (\u5171\u4eab\u5c42\u4e0d\u4f9d\u8d56\u5e73\u53f0)\n\u274c shared/ \u2192 ui/          (\u5171\u4eab\u5c42\u4e0d\u4f9d\u8d56 UI)\n\u274c shared/ \u2192 gpui_app/    (\u5171\u4eab\u5c42\u4e0d\u4f9d\u8d56 GPUI)\n```"
if old6b in content:
    content = content.replace(old6b, new6b)
    changes += 1
    print("6b. forbidden deps updated")
else:
    print("6b. SKIP: forbidden deps pattern not found")

# 7. Add checklist item
old7 = "- [ ] \u5355\u6587\u4ef6\u4e0d\u8d85\u8fc7 1500 \u884c\uff08\u6e32\u67d3\u5f15\u64ce\u7b49\u590d\u6742\u6a21\u5757\u53ef\u9002\u5f53\u653e\u5bbd\uff09"
new7 = "- [ ] \u5982\u679c\u662f\u8de8\u5e73\u53f0\u7eaf\u903b\u8f91\uff0c\u653e\u5165 shared/ \u800c\u975e platform/<os>/\n- [ ] \u5355\u6587\u4ef6\u4e0d\u8d85\u8fc7 1500 \u884c\uff08\u6e32\u67d3\u5f15\u64ce\u7b49\u590d\u6742\u6a21\u5757\u53ef\u9002\u5f53\u653e\u5bbd\uff09"
if old7 in content:
    content = content.replace(old7, new7)
    changes += 1
    print("7. checklist updated")
else:
    print("7. SKIP: checklist pattern not found")

print(f"\nTotal changes: {changes}")

with open(filepath, 'w', encoding='utf-8') as f:
    f.write(content)

print("File written successfully.")