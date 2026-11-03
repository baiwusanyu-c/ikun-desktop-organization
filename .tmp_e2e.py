# -*- coding: utf-8 -*-
"""分类框端到端取证驱动：真实鼠标输入 + 日志/窗口状态核对（纯 ctypes，无第三方依赖）。"""
import ctypes, ctypes.wintypes as wt, io, os, subprocess, sys, time, glob

u = ctypes.windll.user32
k32 = ctypes.windll.kernel32
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
EXE = r"D:\chapanda-project\chapanda-ds-organizers\target\debug\ikun_organizer.exe"
LOGDIR = os.path.join(os.path.expanduser("~"), ".ikun_organizer_log")
MARK = ".tmp_e2e_mark.txt"


class RECT(ctypes.Structure):
    _fields_ = [("l", ctypes.c_long), ("t", ctypes.c_long), ("r", ctypes.c_long), ("b", ctypes.c_long)]


class INPUT(ctypes.Structure):
    class MI(ctypes.Structure):
        _fields_ = [("dx", ctypes.c_long), ("dy", ctypes.c_long), ("mouseData", ctypes.c_ulong),
                    ("dwFlags", ctypes.c_ulong), ("time", ctypes.c_ulong), ("extra", ctypes.c_void_p)]
    _fields_ = [("type", ctypes.c_ulong), ("mi", MI)]


ENUMPROCTYPE = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)


def log_path():
    return max(glob.glob(os.path.join(LOGDIR, ".ikun_organizer_log_*")), key=os.path.getmtime)


def mark():
    with open(MARK, "w") as f:
        f.write(str(os.path.getsize(log_path())))


def tail(skip_debug=True):
    off = int(open(MARK).read()) if os.path.exists(MARK) else 0
    with io.open(log_path(), encoding="utf-8", errors="replace") as f:
        f.seek(off)
        txt = f.read()
    out = [l for l in txt.splitlines() if l.startswith("[2026")]
    if skip_debug:
        out = [l for l in out if " DEBUG " not in l]
    return out


def windows(cls):
    found = []

    def cb(hwnd, _):
        buf = ctypes.create_unicode_buffer(64)
        u.GetClassNameW(hwnd, buf, 64)
        if cls and buf.value != cls:
            return True
        pid = wt.DWORD()
        u.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        r = RECT()
        u.GetWindowRect(hwnd, ctypes.byref(r))
        vis = bool(u.IsWindowVisible(hwnd))
        ex = u.GetWindowLongW(hwnd, -20) & 0xFFFFFFFF
        found.append(dict(hwnd=hwnd, pid=pid.value, cls=buf.value, l=r.l, t=r.t, r=r.r, b=r.b, vis=vis, ex=ex))
        return True

    u.EnumWindows(ENUMPROCTYPE(cb), 0)
    return found


def app_pid():
    w = windows("DesktopOrgGrid")
    return w[0]["pid"] if w else None


def boxes():
    pid = app_pid()
    return [w for w in windows("DesktopOrgGrid") if w["pid"] == pid and w["vis"]], [w for w in windows("LiquidVerreCanvas") if w["pid"] == pid and w["vis"]]


def move(x, y, ms=120):
    u.SetCursorPos(int(x), int(y))
    time.sleep(ms / 1000.0)


def btn(flags, data=0):
    inp = INPUT(type=0)
    inp.mi.dwFlags = flags
    inp.mi.mouseData = data
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def down(): btn(0x0002)
def up(): btn(0x0004)


def click(x, y, hold=90):
    move(x, y, 200)
    down(); time.sleep(hold / 1000.0); up()
    time.sleep(0.5)


def dblclick(x, y):
    move(x, y, 200)
    for _ in range(2):
        down(); time.sleep(0.04); up(); time.sleep(0.06)
    time.sleep(0.8)


def drag(fx, fy, tx, ty, steps=18, dwell=0.03):
    move(fx, fy, 250)
    down()
    time.sleep(0.12)
    for i in range(1, steps + 1):
        t = i / steps
        move(fx + (tx - fx) * t, fy + (ty - fy) * t, int(dwell * 1000))
    time.sleep(0.15)
    up()
    time.sleep(0.8)


def canvas_transparent():
    _, cvs = boxes()
    if not cvs:
        return None
    return bool(cvs[0]["ex"] & 0x20)


def start():
    if app_pid():
        print("already running pid=", app_pid())
        return
    env = dict(os.environ)
    env["RUST_LOG"] = ("warn,ikun_organizer=info,liquid_verre=info,"
                       "ikun_organizer::platform::windows::glass_engine=debug,"
                       "ikun_organizer::platform::windows::grid_window=debug,"
                       "ikun_organizer::platform::windows::wnd_proc=debug")
    subprocess.Popen([EXE], cwd=os.path.dirname(EXE), env=env,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(9)
    print("started pid=", app_pid())


def stop():
    subprocess.run(["taskkill", "/IM", "ikun_organizer.exe", "/F"], capture_output=True)
    time.sleep(2)
    print("stopped")


def show_boxes():
    bs, cvs = boxes()
    for i, b in enumerate(bs):
        print(f"  box[{i}] hwnd=0x{b['hwnd']:X} rect=({b['l']},{b['t']})-({b['r']},{b['b']}) size={b['r']-b['l']}x{b['b']-b['t']}")
    for c in cvs:
        print(f"  canvas hwnd=0x{c['hwnd']:X} ex=0x{c['ex']:X} transparent={bool(c['ex'] & 0x20)}")
    return bs, cvs


def box_local(b, lx, ly):
    return b["l"] + lx, b["t"] + ly


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "win"
    arg = sys.argv[2] if len(sys.argv) > 2 else ""

    def dump():
        for l in tail():
            print("   " + l[:190])

    if cmd == "start":
        start()
    elif cmd == "stop":
        stop()
    elif cmd == "win":
        show_boxes()
    elif cmd == "probe":
        bs, cvs = show_boxes()
        for spec in arg.split(";"):
            i, xy = spec.split(":")
            b = bs[int(i)]
            x, y = box_local(b, *[int(v) for v in xy.split(",")])
            move(x, y, 450)
            print(f"  box{i} local({xy}) -> screen({x},{y}) canvas_transparent={canvas_transparent()}")
        move(2400, 1900, 300)
    elif cmd == "tail":
        dump()
    elif cmd in ("dbl", "click", "rclick", "hover", "drag"):
        bs, _ = boxes()
        parts = arg.split(":")
        b = bs[int(parts[0])]
        mark()
        if cmd == "drag":
            fx, fy = box_local(b, int(parts[1]), int(parts[2]))
            tx, ty = box_local(b, int(parts[3]), int(parts[4]))
            drag(fx, fy, tx, ty)
        else:
            x, y = box_local(b, int(parts[1]), int(parts[2]))
            if cmd == "dbl":
                dblclick(x, y)
            elif cmd == "rclick":
                move(x, y, 250)
                btn(0x0008)
                time.sleep(0.08)
                btn(0x0010)
                time.sleep(0.9)
            elif cmd == "hover":
                move(x, y, 1500)
            else:
                click(x, y)
        print(f"{cmd} box{parts[0]} arg={arg} 完成，新增日志：")
        dump()
    else:
        print("usage: start|stop|win|probe|tail|dbl i:x:y|click i:x:y|rclick i:x:y|hover i:x:y|drag i:x:y:X:Y")
