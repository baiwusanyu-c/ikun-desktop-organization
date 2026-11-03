# -*- coding: utf-8 -*-
"""用可靠锚点标定 Z 序方向，再判定分类框到底在其他窗口之上还是之下。"""
import ctypes
from ctypes import wintypes as wt
u = ctypes.windll.user32
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
exec(open(".tmp_e2e.py", encoding="utf-8").read().split("if __name__ ==")[0])
GWF = 2   # GW_HWNDNEXT
GWPREV = 3

ENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
rows = []


def cb(h, _):
    if not u.IsWindowVisible(h):
        return True
    buf = ctypes.create_unicode_buffer(60)
    u.GetClassNameW(h, buf, 60)
    ex = u.GetWindowLongW(h, -20) & 0xFFFFFFFF
    r = wt.RECT()
    u.GetWindowRect(h, ctypes.byref(r))
    pid = wt.DWORD()
    u.GetWindowThreadProcessId(h, ctypes.byref(pid))
    rows.append(dict(h=h, cls=buf.value, ex=ex, l=r.left, t=r.top, rr=r.right, b=r.bottom, pid=pid.value))
    return True


u.EnumWindows(ENUMPROC(cb), 0)
print("可见顶层窗口数 =", len(rows))


def rank(h):
    """返回该窗口在 EnumWindows 序列里的下标（0 = 第一个被枚举到的）。"""
    for i, rw in enumerate(rows):
        if rw["h"] == h:
            return i
    return None


def cls_of(h):
    buf = ctypes.create_unicode_buffer(60)
    u.GetClassNameW(h, buf, 60)
    return buf.value


# 方向标定：从 Progman 沿 NEXT 走，看是否走到「更底」；从 Shell_TrayWnd 沿 PREV 走
prog = [rw for rw in rows if rw["cls"] == "Progman"]
tray = [rw for rw in rows if rw["cls"] == "Shell_TrayWnd"]
if prog:
    p = prog[0]["h"]
    print("Progman 的 NEXT 方向前 3 个:", [cls_of(x) for x in [u.GetWindow(p, GWF)] if x] , "| PREV 方向前 3 个:", [cls_of(x) for x in [u.GetWindow(p, GWPREV)] if x])
if tray:
    t = tray[0]["h"]
    print("Shell_TrayWnd ex=0x%X topmost=%s" % (tray[0]["ex"], bool(tray[0]["ex"] & 0x8)))

print("\nEnumWindows 序列里的关键位置（下标越小越先被枚举）:")
for key in ("Shell_TrayWnd", "Progman", "Chrome_WidgetWin_1", "Qt51514QWindowIcon", "DesktopOrgGrid", "LiquidVerreCanvas"):
    idxs = [i for i, rw in enumerate(rows) if rw["cls"] == key]
    print("   %-24s -> %s" % (key, idxs[:8]))
