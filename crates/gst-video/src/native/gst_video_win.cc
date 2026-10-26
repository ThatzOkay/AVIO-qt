// Windows video plane.

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <commctrl.h>

#include <cmath>

#include <gst/gst.h>
#include <gst/video/videooverlay.h>

struct AvioVideoView {
  HWND parent;  // the electron top-level window this plane belongs to
  HWND video;   // our borderless plane window (the d3d11videosink renders into this)
  void* sink;
  double cropL, cropT, visW, visH, tierW, tierH;
  int lastW, lastH;
  bool hidden;
};

static const wchar_t* kAvioVideoClass = L"AvioVideoPlane";

static LRESULT CALLBACK avio_plane_proc(HWND h, UINT msg, WPARAM w, LPARAM l) {
  return DefWindowProcW(h, msg, w, l);
}

static void avio_ensure_class() {
  static bool registered = false;
  if (registered) return;
  registered = true;
  WNDCLASSEXW wc;
  ZeroMemory(&wc, sizeof(wc));
  wc.cbSize = sizeof(wc);
  wc.lpfnWndProc = avio_plane_proc;
  wc.hInstance = GetModuleHandleW(NULL);
  wc.hCursor = NULL;
  wc.hbrBackground = (HBRUSH)GetStockObject(BLACK_BRUSH);
  wc.lpszClassName = kAvioVideoClass;
  RegisterClassExW(&wc);
}

static void avio_apply_render_rect(AvioVideoView* v) {
  if (!v || !v->sink || !GST_IS_VIDEO_OVERLAY(v->sink) || !IsWindow(v->video)) return;
  RECT rc;
  if (!GetClientRect(v->video, &rc)) return;
  const double W = rc.right - rc.left;
  const double H = rc.bottom - rc.top;
  if (W <= 0.0 || H <= 0.0) return;
  v->lastW = static_cast<int>(W);
  v->lastH = static_cast<int>(H);

  GstVideoOverlay* ov = GST_VIDEO_OVERLAY(v->sink);
  if (v->visW <= 0.0 || v->visH <= 0.0 || v->tierW <= 0.0 || v->tierH <= 0.0) {
    gst_video_overlay_set_render_rectangle(ov, 0, 0, static_cast<gint>(W), static_cast<gint>(H));
  } else {
    const double scale = std::fmin(W / v->visW, H / v->visH);
    const double cdw = v->visW * scale;
    const double cdh = v->visH * scale;
    const double cx = (W - cdw) / 2.0;
    const double cy = (H - cdh) / 2.0;
    gst_video_overlay_set_render_rectangle(
        ov, static_cast<gint>(std::lround(cx - v->cropL * scale)),
        static_cast<gint>(std::lround(cy - v->cropT * scale)),
        static_cast<gint>(std::lround(v->tierW * scale)),
        static_cast<gint>(std::lround(v->tierH * scale)));
  }
  gst_video_overlay_expose(ov);
}

static void avio_sync(AvioVideoView* v) {
  if (!v || !IsWindow(v->parent) || !IsWindow(v->video)) return;
  RECT cr;
  if (!GetClientRect(v->parent, &cr)) return;
  POINT tl = {cr.left, cr.top};
  ClientToScreen(v->parent, &tl);
  const int x = tl.x;
  const int y = tl.y;
  const int w = cr.right - cr.left;
  const int h = cr.bottom - cr.top;
  UINT flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER;
  flags |= v->hidden ? SWP_HIDEWINDOW : SWP_SHOWWINDOW;
  SetWindowPos(v->video, v->parent, x, y, w, h, flags);
  if (w != v->lastW || h != v->lastH) avio_apply_render_rect(v);
}

// Subclass on the PARENT (electron) window: keep the plane under it
static LRESULT CALLBACK avio_parent_proc(HWND h, UINT msg, WPARAM w, LPARAM l, UINT_PTR id,
                                         DWORD_PTR ref) {
  AvioVideoView* v = reinterpret_cast<AvioVideoView*>(ref);
  switch (msg) {
    case WM_WINDOWPOSCHANGED:
    case WM_MOVE:
    case WM_SIZE:
    case WM_SHOWWINDOW:
    case WM_ACTIVATE:
      avio_sync(v);
      break;
    case WM_NCDESTROY:
      RemoveWindowSubclass(h, avio_parent_proc, id);
      break;
    default:
      break;
  }
  return DefSubclassProc(h, msg, w, l);
}

extern "C" guintptr avio_attach_view(guintptr parent, void** outView) {
  *outView = nullptr;
  HWND parentHwnd = reinterpret_cast<HWND>(static_cast<uintptr_t>(parent));
  if (!parentHwnd || !IsWindow(parentHwnd)) return parent;

  avio_ensure_class();

  RECT cr = {0, 0, 0, 0};
  GetClientRect(parentHwnd, &cr);
  POINT tl = {cr.left, cr.top};
  ClientToScreen(parentHwnd, &tl);

  HWND video = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
                               kAvioVideoClass, L"", WS_POPUP, tl.x, tl.y,
                               cr.right - cr.left, cr.bottom - cr.top, NULL, NULL,
                               GetModuleHandleW(NULL), NULL);
  if (!video) return parent;

  AvioVideoView* v = new AvioVideoView();
  v->parent = parentHwnd;
  v->video = video;
  v->sink = nullptr;
  v->cropL = v->cropT = v->visW = v->visH = v->tierW = v->tierH = 0;
  v->lastW = v->lastH = 0;
  v->hidden = false;

  SetWindowSubclass(parentHwnd, avio_parent_proc, reinterpret_cast<UINT_PTR>(v),
                    reinterpret_cast<DWORD_PTR>(v));
  avio_sync(v);

  *outView = v;
  return reinterpret_cast<guintptr>(video);  // the sink renders into the plane window
}

extern "C" void avio_set_view_hidden(void* view, bool hidden) {
  AvioVideoView* v = reinterpret_cast<AvioVideoView*>(view);
  if (!v || !IsWindow(v->video)) return;
  v->hidden = hidden;
  ShowWindow(v->video, hidden ? SW_HIDE : SW_SHOWNOACTIVATE);
  if (!hidden) avio_sync(v);  // re-pin under the parent when re-shown
}

// Set the content region (crop offsets + visible size within the decoded tier)
extern "C" void avio_set_content_region(void* view, void* sink, double cropL, double cropT,
                                        double visW, double visH, double tierW, double tierH) {
  AvioVideoView* v = reinterpret_cast<AvioVideoView*>(view);
  if (!v) return;
  v->sink = sink;
  v->cropL = cropL;
  v->cropT = cropT;
  v->visW = visW;
  v->visH = visH;
  v->tierW = tierW;
  v->tierH = tierH;
  v->lastW = v->lastH = 0;
  avio_apply_render_rect(v);
}

extern "C" void avio_remove_view(void* view) {
  AvioVideoView* v = reinterpret_cast<AvioVideoView*>(view);
  if (!v) return;
  if (IsWindow(v->parent)) {
    RemoveWindowSubclass(v->parent, avio_parent_proc, reinterpret_cast<UINT_PTR>(v));
  }
  if (IsWindow(v->video)) DestroyWindow(v->video);
  delete v;
}

extern "C" void avio_set_backdrop(guintptr parent, double r, double g, double b) {
  (void)parent;
  (void)r;
  (void)g;
  (void)b;
}
