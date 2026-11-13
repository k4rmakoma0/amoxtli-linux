#!/usr/bin/env python3
"""Exercise the installed, strictly confined Snap on an isolated X11 display.

Start Xvfb separately and pass its display; never point this at your desktop.
The test uses only scratch documents and refuses to run while Amoxtli is open.
"""

import argparse
import ctypes as C
import os
from pathlib import Path
import signal
import subprocess
import time


class WindowAttributes(C.Structure):
    _fields_ = [(n, C.c_int) for n in ("x", "y", "width", "height", "border_width", "depth")] + [
        ("visual", C.c_void_p), ("root", C.c_ulong), ("class_", C.c_int),
        ("bit_gravity", C.c_int), ("win_gravity", C.c_int), ("backing_store", C.c_int),
        ("backing_planes", C.c_ulong), ("backing_pixel", C.c_ulong), ("save_under", C.c_int),
        ("colormap", C.c_ulong), ("map_installed", C.c_int), ("map_state", C.c_int),
        ("all_event_masks", C.c_long), ("your_event_mask", C.c_long),
        ("do_not_propagate_mask", C.c_long), ("override_redirect", C.c_int), ("screen", C.c_void_p),
    ]


def wait_for(predicate, description, timeout=20):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError(f"Timed out: {description}")


class Keyboard:
    def __init__(self, display):
        self.x = C.CDLL("libX11.so.6")
        self.xt = C.CDLL("libXtst.so.6")
        self.x.XOpenDisplay.argtypes = [C.c_char_p]
        self.x.XOpenDisplay.restype = C.c_void_p
        self.x.XDefaultRootWindow.argtypes = [C.c_void_p]
        self.x.XDefaultRootWindow.restype = C.c_ulong
        self.x.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong),
                                    C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]
        self.x.XFetchName.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_void_p)]
        self.x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
        self.x.XInternAtom.restype = C.c_ulong
        self.x.XGetWindowAttributes.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(WindowAttributes)]
        self.x.XGetWindowProperty.argtypes = [C.c_void_p, C.c_ulong, C.c_ulong, C.c_long, C.c_long,
                                            C.c_int, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_int),
                                            C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.c_void_p)]
        self.x.XFree.argtypes = [C.c_void_p]
        self.x.XStringToKeysym.argtypes = [C.c_char_p]
        self.x.XStringToKeysym.restype = C.c_ulong
        self.x.XKeysymToKeycode.argtypes = [C.c_void_p, C.c_ulong]
        self.x.XKeysymToKeycode.restype = C.c_uint
        self.x.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
        self.x.XFlush.argtypes = [C.c_void_p]
        self.x.XCloseDisplay.argtypes = [C.c_void_p]
        self.xt.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        self.display = self.x.XOpenDisplay(display.encode())
        if not self.display:
            raise RuntimeError(f"Cannot open test display {display}")

    def window(self, needle):
        root = self.x.XDefaultRootWindow(self.display)
        root_out, parent = C.c_ulong(), C.c_ulong()
        children, count = C.POINTER(C.c_ulong)(), C.c_uint()
        if not self.x.XQueryTree(self.display, root, C.byref(root_out), C.byref(parent), C.byref(children), C.byref(count)):
            return None
        try:
            for i in range(count.value):
                attributes = WindowAttributes()
                if not self.x.XGetWindowAttributes(self.display, children[i], C.byref(attributes)) or attributes.map_state != 2:
                    continue
                name = C.c_void_p()
                atom = self.x.XInternAtom(self.display, b"_NET_WM_NAME", 1)
                actual, count_bytes, remaining, fmt = C.c_ulong(), C.c_ulong(), C.c_ulong(), C.c_int()
                if atom:
                    self.x.XGetWindowProperty(self.display, children[i], atom, 0, 1024, 0, 0,
                                             C.byref(actual), C.byref(fmt), C.byref(count_bytes),
                                             C.byref(remaining), C.byref(name))
                    if name.value:
                        try:
                            if needle in C.string_at(name, count_bytes.value).decode(errors="replace"):
                                return children[i]
                        finally:
                            self.x.XFree(name)
                    name = C.c_void_p()
                if self.x.XFetchName(self.display, children[i], C.byref(name)) and name.value:
                    try:
                        if needle in C.string_at(name).decode(errors="replace"):
                            return children[i]
                    finally:
                        self.x.XFree(name)
        finally:
            self.x.XFree(children)
        return None

    def focus(self, window):
        self.x.XSetInputFocus(self.display, window, 2, 0)
        self.x.XFlush(self.display)

    def key(self, name, *modifiers):
        names = [*modifiers, name]
        codes = [self.x.XKeysymToKeycode(self.display, self.x.XStringToKeysym(n.encode())) for n in names]
        assert all(codes), names
        for code in codes:
            self.xt.XTestFakeKeyEvent(self.display, code, 1, 0)
        for code in reversed(codes):
            self.xt.XTestFakeKeyEvent(self.display, code, 0, 0)
        self.x.XFlush(self.display)
        time.sleep(0.025)

    def type(self, text):
        for char in text:
            self.key("space" if char == " " else char)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--display", required=True, help="An isolated Xvfb display")
    args = parser.parse_args()
    if args.display == os.environ.get("DISPLAY"):
        parser.error("Use a separate Xvfb display, not the current desktop")
    if subprocess.run(["pgrep", "-u", str(os.getuid()), "-x", "amoxtli"], stdout=subprocess.DEVNULL).returncode == 0:
        parser.error("Close existing Amoxtli processes before running the test")
    subprocess.run(["snap", "list", "amoxtli"], check=True, stdout=subprocess.DEVNULL)
    root = Path(__file__).resolve().parents[1] / "target/snap-validation"
    root.mkdir(parents=True, exist_ok=True)
    doc = root / "snap-smoke.md"
    initial = "# snap test\n\noriginal ñ😀\n"
    doc.write_text(initial)
    keyboard = Keyboard(args.display)
    env = {**os.environ, "DISPLAY": args.display, "GDK_BACKEND": "x11", "GSK_RENDERER": "cairo"}
    process = None
    log = (root / "gui-smoke.log").open("w")
    try:
        def launch(path):
            nonlocal process
            process = subprocess.Popen(["snap", "run", "amoxtli", str(path)], env=env,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            # Ignore GTK's unmapped helper window and match the real toplevel.
            wait_for(lambda: keyboard.window("Amoxtli"), "application window")
            keyboard.focus(keyboard.window("Amoxtli"))

        def mode(number):
            keyboard.key(number, "Control_L", "Alt_L")
            time.sleep(0.3)
            keyboard.key("End", "Control_L")

        launch(doc)
        time.sleep(1.2)
        assert doc.read_text() == initial, "Opening a document must not rewrite it"
        print("PASS opening preserves the document", flush=True)
        mode("3")
        keyboard.type("source edit")
        wait_for(lambda: doc.read_text() == initial + "source edit", "source auto-save")
        print("PASS source edit and automatic save", flush=True)
        mode("4")
        keyboard.type(" format edit")
        wait_for(lambda: "format edit" in doc.read_text(), "formatted auto-save")
        assert "source edit" in doc.read_text() and "ñ😀" in doc.read_text()
        print("PASS formatted edit preserves prior edits and Unicode", flush=True)
        mode("3")
        mode("5")
        keyboard.type(" split edit")
        wait_for(lambda: "split edit" in doc.read_text(), "split auto-save")
        print("PASS split edit and automatic save", flush=True)
        mode("3")
        keyboard.type(" pending change")
        doc.write_text("# external change\n")
        time.sleep(1.5)
        assert doc.read_text() == "# external change\n", "External changes were overwritten without confirmation"
        print("PASS external changes are protected until confirmation", flush=True)
        keyboard.key("Escape")  # The conflict dialog's close response is Keep Mine.
        wait_for(lambda: "pending change" in doc.read_text(), "confirmed conflict save")
        keyboard.key("q", "Control_L")
        process.wait(timeout=15)
        assert process.returncode == 0
        persisted = doc.read_text()
        launch(doc)
        time.sleep(1.2)
        assert doc.read_text() == persisted
        keyboard.key("q", "Control_L")
        process.wait(timeout=15)
        print("PASS closing and reopening preserves edits", flush=True)
        folder = root / "notes"
        folder.mkdir(exist_ok=True)
        (folder / "README.md").write_text("# folder test\n")
        launch(folder)
        mode("3")
        keyboard.type("folder edit")
        wait_for(lambda: "folder edit" in (folder / "README.md").read_text(), "folder browser save")
        keyboard.key("q", "Control_L")
        process.wait(timeout=15)
        print("PASS folder opening and saving its README", flush=True)
        print("Snap GUI smoke test: PASS", flush=True)
    finally:
        if process and process.poll() is None:
            window = keyboard.window("Amoxtli")
            if window:
                keyboard.focus(window)
                keyboard.key("Escape")
                keyboard.key("s", "Control_L")
                time.sleep(1.2)
                keyboard.key("q", "Control_L")
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                    process.wait(timeout=5)
                except PermissionError:
                    print("Close the remaining scratch window on the test display.", flush=True)
        log.close()
        keyboard.x.XCloseDisplay(keyboard.display)


if __name__ == "__main__":
    main()
