"""精确测量：画布下方压着多少个第三方应用窗口（idx 更大 = 更低 = 被玻璃盖住）。"""
import ctypes, ctypes.wintypes as wt, os, subprocess, time
u = ctypes.windll.user32
EXE = r"D:\chapanda-project\chapanda-ds-organizers\target\debug\ikun_organizer.exe"
SHELL = {"Progman", "WorkerW", "Shell_TrayWnd", "NotifyIconOverflowWindow",
         "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow",
         "TopLevelWindowForOverflowXamlIsland", "MsgrIMEWindowClass",
         "Default IME", "MSCTFIME UI", "IntfUI", "SysDragImage",
         "XamlExplorerHostIslandWindow", "WindowsInputManager_XInputApp"}
mine = {"LiquidVerreCanvas", "DesktopOrgGrid", "Tauri Window", "Chrome_WidgetWin_1"}


def rows_all():
    rows = []
    idx = [0]
    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(h, lp):
        i = idx[0]; idx[0] += 1
        if not u.IsWindowVisible(h):
            return True
        c = ctypes.create_unicode_buffer(128); u.GetClassNameW(h, c, 128)
        t = ctypes.create_unicode_buffer(128); u.GetWindowTextW(h, t, 128)
        pid = wt.DWORD(); u.GetWindowThreadProcessId(h, ctypes.byref(pid))
        r = wt.RECT(); u.GetWindowRect(h, ctypes.byref(r))
        rows.append((i, c.value, t.value, pid.value, r.left, r.top, r.right, r.bottom))
        return True
    u.EnumWindows(cb, 0)
    return rows


def big(l, t, rr, b):
    return l > -30000 and (rr - l) > 100 and (b - t) > 100


subprocess.run(["taskkill", "/IM", "ikun_organizer.exe", "/F"], capture_output=True)
time.sleep(1.5)
base = [(i, c, t) for i, c, t, p, l, to, rr, b in rows_all()
        if c not in SHELL and big(l, to, rr, b) and t]
print("启动前的可见应用窗口：")
for i, c, t in base[:10]:
    print("   idx=%-4s %-26s %s" % (i, c[:26], t[:40]))
t0 = time.time()
subprocess.Popen([EXE], cwd=os.path.dirname(EXE), env={k: v for k, v in os.environ.items() if k != "RUST_LOG"},
                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
worst = None
cover_samples = 0
samples = 0
seen = False
while time.time() - t0 < 12:
    samples += 1
    rows = rows_all()
    ci = min((i for i, c, *_ in rows if c == "LiquidVerreCanvas"), default=None)
    if ci is not None and not seen:
        seen = True
        print("首块画布可见 t+%.3fs idx=%s" % (time.time() - t0, ci))
    if ci is not None:
        bad = [(c, t) for i, c, t, p, l, to, rr, b in rows
               if i > ci and c not in SHELL and t and big(l, to, rr, b)
               and (c, t) in {(c2, t2) for _, c2, t2 in base}]
        if bad:
            cover_samples += 1
            if worst is None or len(bad) > len(worst[1]):
                worst = (time.time() - t0, bad)
    time.sleep(0.004)
print("采样=%s 盖住基线窗口的采样=%s" % (samples, cover_samples))
if worst:
    print("最严重 t+%.3fs: %s" % (worst[0], worst[1][:5]))
