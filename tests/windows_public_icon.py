"""Verify the Windows icon and a public HTTPS page in a new Flit profile."""
import argparse
import ctypes as C
from ctypes import wintypes as W
from hashlib import sha256
import json
from pathlib import Path
import tempfile

from windows_smoke import Browser, OwnedProcess, WORKSPACE, Win32, api, require

URL = "https://example.com/"


def inspect_icons(exe, ico, hwnd):
    user = C.WinDLL("user32", use_last_error=True)
    gdi = C.WinDLL("gdi32", use_last_error=True)
    kernel = C.WinDLL("kernel32", use_last_error=True)
    ptr = C.c_void_p
    load_module = api(kernel, "LoadLibraryExW", W.HMODULE, W.LPCWSTR, W.HANDLE, W.DWORD)
    free_module = api(kernel, "FreeLibrary", W.BOOL, W.HMODULE)
    load_image = api(user, "LoadImageW", W.HANDLE, W.HINSTANCE, ptr, W.UINT, C.c_int, C.c_int, W.UINT)
    class_long = api(user, "GetClassLongPtrW", C.c_size_t, W.HWND, C.c_int)
    destroy_icon = api(user, "DestroyIcon", W.BOOL, W.HICON)
    class IconInfo(C.Structure):
        _fields_ = [("icon", W.BOOL), ("x", W.DWORD), ("y", W.DWORD),
                    ("mask", W.HBITMAP), ("color", W.HBITMAP)]

    class Bitmap(C.Structure):
        _fields_ = [("kind", W.LONG), ("width", W.LONG), ("height", W.LONG),
                    ("width_bytes", W.LONG), ("planes", W.WORD), ("bits_pixel", W.WORD),
                    ("bits", ptr)]

    icon_info = api(user, "GetIconInfo", W.BOOL, W.HICON, C.POINTER(IconInfo))
    object_info = api(gdi, "GetObjectW", C.c_int, W.HANDLE, C.c_int, ptr)
    dc_new = api(gdi, "CreateCompatibleDC", W.HDC, W.HDC)
    dc_delete = api(gdi, "DeleteDC", W.BOOL, W.HDC)
    dib_new = api(gdi, "CreateDIBSection", W.HBITMAP, W.HDC, ptr, W.UINT, C.POINTER(ptr), W.HANDLE, W.DWORD)
    select = api(gdi, "SelectObject", W.HANDLE, W.HDC, W.HANDLE)
    delete = api(gdi, "DeleteObject", W.BOOL, W.HANDLE)
    draw = api(user, "DrawIconEx", W.BOOL, W.HDC, C.c_int, C.c_int, W.HICON, C.c_int, C.c_int, W.UINT, W.HBRUSH, W.UINT)

    def size(icon):
        info = IconInfo()
        Win32.check(icon_info(icon, C.byref(info)))
        try:
            bitmap = Bitmap()
            Win32.check(object_info(info.color or info.mask, C.sizeof(bitmap), C.byref(bitmap)))
            return bitmap.width, bitmap.height if info.color else bitmap.height // 2
        finally:
            for handle in (info.mask, info.color):
                if handle:
                    delete(handle)

    def render(icon, width, height):
        import struct

        buffer = ptr()
        header = C.create_string_buffer(struct.pack("<IiiHHIIiiII", 40, width, -height, 1, 32, 0, 0, 0, 0, 0, 0))
        dc = Win32.check(dc_new(None))
        bitmap = previous = None
        try:
            bitmap = Win32.check(dib_new(dc, C.cast(header, ptr), 0, C.byref(buffer), None, 0))
            previous = Win32.check(select(dc, bitmap))
            C.memset(buffer, 0, width * height * 4)
            Win32.check(draw(dc, 0, 0, icon, width, height, 0, None, 3))
            raw = C.string_at(buffer, width * height * 4)
            # DrawIconEx paints BGR; the fourth byte of this BI_RGB DIB is not a color channel.
            return b"".join(raw[i:i + 3] for i in range(0, len(raw), 4))
        finally:
            if previous:
                select(dc, previous)
            if bitmap:
                delete(bitmap)
            dc_delete(dc)

    record = {}
    module = Win32.check(load_module(str(exe), None, 0x22))  # Resource-only; do not execute it.
    try:
        filename = C.create_unicode_buffer(str(ico))
        for kind, class_index in [("large", -14), ("small", -34)]:
            source_icon = pe_icon = None
            try:
                window_icon = Win32.check(class_long(hwnd, class_index))
                width, height = size(window_icon)
                require(0 < width <= 256 and 0 < height <= 256, "Invalid live icon dimensions")
                source_icon = Win32.check(load_image(None, C.cast(filename, ptr), 1, width, height, 0x10))
                pe_icon = Win32.check(load_image(module, ptr(32512), 1, width, height, 0))
                source = render(source_icon, width, height)
                pe_pixels = render(pe_icon, width, height)
                live_pixels = render(window_icon, width, height)
                require(pe_pixels == source, f"{kind} EXE icon differs from Flit.ico")
                different = sum(a != b for a, b in zip(live_pixels, source))
                require(live_pixels == source,
                        f"{kind} live window icon differs from Flit.ico ({different}/{len(source)} BGR bytes)")
                require(len(set(source)) > 8, f"{kind} icon is blank")
                record[kind] = {"width": width, "height": height, "pixels_sha256": sha256(source).hexdigest()}
            finally:
                if source_icon:
                    destroy_icon(source_icon)
                if pe_icon:
                    destroy_icon(pe_icon)
    finally:
        free_module(module)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--ico", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    exe, ico, work = (p.resolve(strict=True) for p in (args.exe, args.ico, args.work))
    require(work.is_relative_to(WORKSPACE), "Use an isolated workspace directory.")
    require(not args.evidence.exists(), "Choose a new evidence file.")
    profile = Path(tempfile.mkdtemp(prefix="flit-public-", dir=work)) / "profile"
    launch = {}
    evidence = {"result": "fail", "launch": launch}
    try:
        with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}", f"--flit-url={URL}"], launch) as process:
            browser = Browser(process, None)
            evidence["icons"] = inspect_icons(exe, ico, browser.hwnd)
            evidence["https"] = {"url": URL, "status": "unverified"}
            browser.wait("public HTTPS page address and committed document title",
                         lambda: browser.address() == URL and browser.has_page_title("Example Domain"), timeout=25)
            evidence["https"] = {"url": URL, "title": browser.title(), "status": "observed"}
            process.normal_close(browser.hwnd)
        evidence["result"] = "pass"
    except (OSError, RuntimeError, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    with args.evidence.open("x", encoding="utf-8") as output:
        json.dump(evidence, output, indent=2)
        output.write("\n")
    print(json.dumps(evidence))
    return 0 if evidence["result"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
