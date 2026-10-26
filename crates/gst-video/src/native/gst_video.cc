#include "gst-video/src/native/gst_video.h"
#include "gst-video/src/lib.rs.h"

#include <gst/app/gstappsrc.h>
#include <gst/base/gstbasesink.h>
#include <gst/video/videooverlay.h>
#include <gst/video/video.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <string>
#ifdef __linux__
#include <execinfo.h>
#include <fcntl.h>
#include <glib-unix.h>
#include <signal.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#endif

static void ensure_init() {
  static bool done = false;
  if (!done) {
    gst_init(NULL, NULL);
    if (const char* dbg = getenv("AVIO_GST_DEBUG")) {
      gst_debug_set_threshold_from_string(
        (dbg[0] == '1' && dbg[1] == '\0')
          ? "v4l2codecs-decoder:6,v4l2codecs-h265dec:6,waylandsink:5,wl_dmabuf:6"
          : dbg,
        FALSE);
    }
    done = true;
  }
}

static GstBusSyncReply bus_sync(GstBus*, GstMessage* msg, gpointer user_data) {
  Player* p = static_cast<Player*>(user_data);
  GstMessageType t = GST_MESSAGE_TYPE(msg);
  if (t == GST_MESSAGE_ERROR) {
    // The sink losing its output surface (e.g. the user closed a bare waylandsink window on a
    // desktop with no avio-compositor to prevent that) is an expected close, not a real error:
    // mark the player dead so it's torn down quietly instead of spamming the error path below.
    // Never touch pipeline state here — this runs synchronously on the streaming thread that
    // posted the message, and a state change here could deadlock.
    if (p && p->sink && msg->src == GST_OBJECT_CAST(p->sink)) {
      fprintf(stderr, "[gst_video] sink closed, tearing down player\n");
      p->dead.store(true);
      return GST_BUS_PASS;
    }
    GError* e = nullptr; gchar* d = nullptr;
    gst_message_parse_error(msg, &e, &d);
    fprintf(stderr, "[gst_video] ERROR from %s: %s | %s\n",
      GST_OBJECT_NAME(msg->src), e ? e->message : "?", d ? d : "");
    if (e) g_error_free(e);
    g_free(d);
  } else if (t == GST_MESSAGE_WARNING) {
    GError* e = nullptr; gchar* d = nullptr;
    gst_message_parse_warning(msg, &e, &d);
    fprintf(stderr, "[gst_video] WARN from %s: %s | %s\n",
      GST_OBJECT_NAME(msg->src), e ? e->message : "?", d ? d : "");
    if (e) g_error_free(e);
    g_free(d);
  }
  return GST_BUS_PASS;
}

// Force every base sink in the pipeline to render unsynced (live, drop-late)
static void force_sinks_realtime(GstElement* pipeline) {
  GstIterator* it = gst_bin_iterate_recurse(GST_BIN(pipeline));
  GValue item = G_VALUE_INIT;
  gboolean done = FALSE;
  while (!done) {
    switch (gst_iterator_next(it, &item)) {
      case GST_ITERATOR_OK: {
        GstElement* el = GST_ELEMENT(g_value_get_object(&item));
        if (GST_IS_BASE_SINK(el)) {
          g_object_set(el, "sync", FALSE, "qos", FALSE, "max-lateness", (gint64)0, NULL);
        }
        g_value_reset(&item);
        break;
      }
      case GST_ITERATOR_RESYNC:
        gst_iterator_resync(it);
        break;
      case GST_ITERATOR_ERROR:
      case GST_ITERATOR_DONE:
        done = TRUE;
        break;
    }
  }
  g_value_unset(&item);
  gst_iterator_free(it);
}

// The Pi 4 stateful v4l2 decoder only accepts one specific colorimetry and rejects any other
// value a phone's H.264 stream might declare (seen in practice: "1:4:5:1" and "2:4:16:3", the
// latter's transfer=16 being SMPTE ST 2084/PQ HDR metadata a basic decoder won't recognize).
// Since this is metadata only (doesn't affect actual pixel decoding), normalize anything that
// isn't already the known-good value instead of matching specific bad ones.
static const char* kGoodColorimetry = "1:4:7:1";

static const char* caps_colorimetry(GstCaps* caps) {
  if (!caps || gst_caps_get_size(caps) == 0) return nullptr;
  return gst_structure_get_string(gst_caps_get_structure(caps, 0), "colorimetry");
}

static bool needs_colorimetry_fixup(const char* col) {
  return col != nullptr && strcmp(col, kGoodColorimetry) != 0;
}

static GstPadProbeReturn colorimetry_query_probe(GstPad* pad, GstPadProbeInfo* info, gpointer) {
  GstQuery* q = GST_PAD_PROBE_INFO_QUERY(info);
  if (!q) return GST_PAD_PROBE_OK;
  if (GST_QUERY_TYPE(q) == GST_QUERY_CAPS) {
    GstCaps* filter = nullptr;
    gst_query_parse_caps(q, &filter);
    if (!needs_colorimetry_fixup(caps_colorimetry(filter))) return GST_PAD_PROBE_OK;
    GstCaps* tmpl = gst_pad_get_pad_template_caps(pad);
    GstCaps* res = gst_caps_intersect(tmpl, filter);
    gst_caps_unref(tmpl);
    gst_query_set_caps_result(q, res);
    gst_caps_unref(res);
    return GST_PAD_PROBE_HANDLED;
  }
  if (GST_QUERY_TYPE(q) == GST_QUERY_ACCEPT_CAPS) {
    GstCaps* caps = nullptr;
    gst_query_parse_accept_caps(q, &caps);
    if (!needs_colorimetry_fixup(caps_colorimetry(caps))) return GST_PAD_PROBE_OK;
    gst_query_set_accept_caps_result(q, TRUE);
    return GST_PAD_PROBE_HANDLED;
  }
  return GST_PAD_PROBE_OK;
}

// Rewrite any non-good colorimetry to kGoodColorimetry on the caps event the decoder sees.
static GstPadProbeReturn colorimetry_fixup_probe(GstPad*, GstPadProbeInfo* info, gpointer) {
  GstEvent* ev = GST_PAD_PROBE_INFO_EVENT(info);
  if (!ev || GST_EVENT_TYPE(ev) != GST_EVENT_CAPS) return GST_PAD_PROBE_OK;
  GstCaps* caps = nullptr;
  gst_event_parse_caps(ev, &caps);
  const char* col = caps_colorimetry(caps);
  if (!needs_colorimetry_fixup(col)) return GST_PAD_PROBE_OK;
  GstCaps* nc = gst_caps_copy(caps);
  gst_caps_set_simple(nc, "colorimetry", G_TYPE_STRING, kGoodColorimetry, NULL);
  gst_event_unref(ev);
  GST_PAD_PROBE_INFO_DATA(info) = gst_event_new_caps(nc);
  gst_caps_unref(nc);
  fprintf(stderr, "[gst_video] colorimetry %s -> %s (pi4 v4l2 decoder)\n", col, kGoodColorimetry);
  return GST_PAD_PROBE_OK;
}

static void remove_video_view(Player* p) {
  if (p->view) {
    avio_remove_view(p->view);
    p->view = nullptr;
  }
}

static const char* parser_for(const std::string& c) {
  if (c == "h265") return "h265parse";
  if (c == "vp9") return "vp9parse";
  if (c == "av1") return "av1parse";
  return "h264parse";
}

// First decoder in the list whose factory is registered, falls back to the last entry (software)
static const char* pick_decoder(std::initializer_list<const char*> cands) {
  const char* last = "";
  for (const char* c : cands) {
    last = c;
    GstElementFactory* f = gst_element_factory_find(c);
    if (f) {
      gst_object_unref(f);
      return c;
    }
  }
  return last;
}

// Software decoders (everything else, vtdec/v4l2*/va*/d3d11*, is HW)
static bool is_hw_decoder(const char* name) {
  if (!name || !*name) return false;
  if (strncmp(name, "avdec_", 6) == 0) return false;
  if (strcmp(name, "vp9dec") == 0 || strcmp(name, "vp8dec") == 0) return false;
  if (strcmp(name, "dav1ddec") == 0 || strcmp(name, "openh264dec") == 0) return false;
  return true;
}

static bool factory_exists(const char* name) {
  GstElementFactory* f = name && *name ? gst_element_factory_find(name) : nullptr;
  if (f) {
    gst_object_unref(f);
    return true;
  }
  return false;
}

// Primary software decoder per codec, used to report SW availability
static const char* sw_decoder_for(const std::string& c) {
  if (c == "h265") return "avdec_h265";
  if (c == "vp9") return "vp9dec";
  if (c == "av1") return "dav1ddec";
  return "avdec_h264";
}

// Best available decoder per codec, HW-first then software fallback
static const char* decoder_for(const std::string& c) {
  if (getenv("AVIO_GST_SWDEC")) {
    if (c == "h265") return "avdec_h265";
    if (c == "vp9") return "vp9dec";
    if (c == "av1") return "dav1ddec";
    return "avdec_h264";
  }
#ifdef __APPLE__
  // HEVC on macOS uses avdec_h265, not vtdec: vtdec ignores sps_max_num_reorder_pics and adds
  // output latency. Revert to vtdec once the GStreamer bug is fixed.
  // https://gitlab.freedesktop.org/gstreamer/gstreamer/-/work_items/5133
  if (c == "h265") return pick_decoder({"avdec_h265", "vtdec"});
  if (c == "vp9") return pick_decoder({"vp9dec"});
  if (c == "av1") return pick_decoder({"dav1ddec"});
  return pick_decoder({"vtdec", "avdec_h264"});
#elif defined(_WIN32)
  if (c == "h265") return pick_decoder({"d3d11h265dec", "avdec_h265"});
  if (c == "vp9") return pick_decoder({"d3d11vp9dec", "vp9dec"});
  if (c == "av1") return pick_decoder({"d3d11av1dec", "dav1ddec"});
  return pick_decoder({"d3d11h264dec", "avdec_h264"});
#else
  if (c == "h265") return pick_decoder({"v4l2slh265dec", "v4l2h265dec", "vah265dec", "avdec_h265"});
  if (c == "vp9") return pick_decoder({"v4l2slvp9dec", "v4l2vp9dec", "vavp9dec", "vp9dec"});
  if (c == "av1") return pick_decoder({"vaav1dec", "dav1ddec"});
  return pick_decoder({"v4l2slh264dec", "v4l2h264dec", "vah264dec", "avdec_h264"});
#endif
}

// Sink chain per platform. Linux presents the decoded dmabuf to avio-compositor via
// waylandsink. mac/Windows render into the window surface directly.
static std::string sink_chain() {
#ifdef __APPLE__
  // force-aspect-ratio=false: the clip view enforces AR, glimagesink must fill (no black bars).
  return "glimagesink name=sink sync=false qos=false force-aspect-ratio=false";
#elif defined(_WIN32)
  return "d3d11videosink name=sink sync=false qos=false force-aspect-ratio=false";
#else
  // waylandsink hands the decoded dmabuf to avio-compositor zero-copy. AVIO_GST_SINK overrides.
  const char* sink_env = getenv("AVIO_GST_SINK");
  return std::string(sink_env && *sink_env ? sink_env : "waylandsink") +
    " name=sink sync=false";
#endif
}

static std::string caps_for(const std::string& c) {
  if (c == "h265") return "video/x-h265,stream-format=byte-stream";
  if (c == "vp9") return "video/x-vp9";
  if (c == "av1") return "video/x-av1";
  return "video/x-h264,stream-format=byte-stream";
}

static void avio_push_player(Player* p, const void* data, size_t len) {
  if (!p || !p->appsrc || !data || len == 0) return;
  GstBuffer* buf = gst_buffer_new_memdup(data, len);
  gst_app_src_push_buffer(GST_APP_SRC(p->appsrc), buf);
}

static const char* CAL_FRAGMENT =
  "#version 100\n"
  "precision highp float;\n"
  "varying vec2 v_texcoord;\n"
  "uniform sampler2D tex;\n"
  "uniform float u_gamma;\n"
  "uniform float u_contrast;\n"
  "uniform float u_gain_r;\n"
  "uniform float u_gain_g;\n"
  "uniform float u_gain_b;\n"
  "void main() {\n"
  "  vec3 c = texture2D(tex, v_texcoord).rgb;\n"
  "  c = pow(c, vec3(1.0 / u_gamma));\n"
  "  c = (c - 0.5) * u_contrast + 0.5;\n"
  "  c = c * vec3(u_gain_r, u_gain_g, u_gain_b);\n"
  "  gl_FragColor = vec4(clamp(c, 0.0, 1.0), 1.0);\n"
  "}\n";

static void avio_set_gamma_player(Player* p, double gamma, double contrast, double gain_r,
                                  double gain_g, double gain_b) {
  if (!p || !p->glshader) return;
  GstStructure* u = gst_structure_new(
    "uniforms", "u_gamma", G_TYPE_FLOAT, (float)(gamma > 0.0 ? gamma : 1.0), "u_contrast",
    G_TYPE_FLOAT, (float)contrast, "u_gain_r", G_TYPE_FLOAT, (float)gain_r, "u_gain_g",
    G_TYPE_FLOAT, (float)gain_g, "u_gain_b", G_TYPE_FLOAT, (float)gain_b, NULL);
  g_object_set(p->glshader, "uniforms", u, NULL);
  gst_structure_free(u);
}

// Build the decode + waylandsink pipeline for a codec. handle is the native window for the
// mac/Windows overlay, unused on Linux. Returns NULL on parse failure.
static Player* avio_create_player(const std::string& codec, guintptr handle) {
  // Two queues: before the decoder non-leaky (a stateless HW decoder needs every
  // frame for its reference chain), after the decoder leaky=downstream.
  const char* decoder = decoder_for(codec);

  std::string presink;
#if !defined(__APPLE__) && !defined(_WIN32)
  if (!is_hw_decoder(decoder)) presink = "videoconvert ! ";
#endif

  std::string cal;
#if defined(__APPLE__)
  cal = "glupload ! glcolorconvert ! glshader name=cal ! ";
#endif

  auto make_desc = [&](const std::string& calc) -> std::string {
    return "appsrc name=src is-live=true do-timestamp=true format=time"
      " min-latency=0 max-latency=0 caps=" +
      caps_for(codec) + " ! " + parser_for(codec) +
      " ! queue max-size-buffers=0 max-size-bytes=0 max-size-time=2000000000" +
      " ! " + std::string(decoder) + " name=dec" +
      " ! queue max-size-buffers=2 max-size-bytes=0 max-size-time=0 leaky=downstream" +
      " ! " + presink + calc + sink_chain();
  };

  std::string desc = make_desc(cal);
  fprintf(stderr, "[gst_video] codec=%s decoder=%s | %s\n",
    codec.c_str(), decoder, desc.c_str());

  GError* err = nullptr;
  GstElement* pipeline = gst_parse_launch(desc.c_str(), &err);
  if ((!pipeline || err) && !cal.empty()) {
    fprintf(stderr, "[gst_video] calibration pass failed (%s), retrying without it\n",
      err ? err->message : "unknown");
    if (err) { g_error_free(err); err = nullptr; }
    if (pipeline) { gst_object_unref(pipeline); pipeline = nullptr; }
    desc = make_desc("");
    pipeline = gst_parse_launch(desc.c_str(), &err);
  }
  if (!pipeline || err) {
    fprintf(stderr, "[gst_video] pipeline parse FAILED: %s\n",
      err ? err->message : "unknown");
    if (err) g_error_free(err);
    if (pipeline) gst_object_unref(pipeline);
    return nullptr;
  }

  Player* p = new Player();
  p->pipeline = pipeline;
  p->appsrc = gst_bin_get_by_name(GST_BIN(pipeline), "src");
  p->sink = gst_bin_get_by_name(GST_BIN(pipeline), "sink");
#ifdef __linux__
  if (p->sink && !getenv("AVIO_COMPOSITOR_CTRL")) {
    // No external compositor running to embed/crop this bare waylandsink surface (e.g. desktop
    // dev/testing) — ask it to go fullscreen directly via the standard Wayland protocol instead
    // of showing at the decoded frame's native size. When avio-compositor IS present, it owns
    // placement/cropping via claim+videocfg, so this only applies without it.
    g_object_set(p->sink, "fullscreen", TRUE, NULL);
  }
#endif
  p->glshader = gst_bin_get_by_name(GST_BIN(pipeline), "cal");
  if (p->glshader) {
    g_object_set(p->glshader, "fragment", CAL_FRAGMENT, NULL);
    avio_set_gamma_player(p, 1.0, 1.0, 1.0, 1.0, 1.0);
  }

  force_sinks_realtime(pipeline);

  GstElement* dec = gst_bin_get_by_name(GST_BIN(pipeline), "dec");
  if (dec) {
    GstPad* dsp = gst_element_get_static_pad(dec, "sink");
    if (dsp) {
      // Only the Pi 4 stateful v4l2 decoders reject 1:4:5:1, the Pi 5 stateless ones accept it.
      if (!strcmp(decoder, "v4l2h264dec") || !strcmp(decoder, "v4l2h265dec")) {
        gst_pad_add_probe(dsp, GST_PAD_PROBE_TYPE_QUERY_DOWNSTREAM, colorimetry_query_probe, NULL,
          NULL);
        gst_pad_add_probe(dsp, GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM, colorimetry_fixup_probe, NULL,
          NULL);
      }
      gst_object_unref(dsp);
    }
    gst_object_unref(dec);
  }

  GstBus* bus = gst_element_get_bus(pipeline);
  gst_bus_set_sync_handler(bus, bus_sync, p, NULL);
  gst_object_unref(bus);

#ifndef __linux__
  guintptr overlay = handle ? avio_attach_view(handle, &p->view) : handle;
  if (p->sink && GST_IS_VIDEO_OVERLAY(p->sink) && overlay) {
    gst_video_overlay_set_window_handle(GST_VIDEO_OVERLAY(p->sink), overlay);
  }
#else
  (void)handle;
#endif

  return p;
}

Player::~Player() {
  if (pipeline) {
    gst_element_set_state(pipeline, GST_STATE_NULL);
    if (appsrc) gst_object_unref(appsrc);
    if (sink) gst_object_unref(sink);
    if (glshader) gst_object_unref(glshader);
    gst_object_unref(pipeline);
  }
  remove_video_view(this);
}

// -- cxx bridge surface -------------------------------------------------------------------

rust::String gv_version() {
  ensure_init();
  gchar* v = gst_version_string();
  rust::String out(v);
  g_free(v);
  return out;
}

rust::Vec<CodecSupport> gv_probe_codecs() {
  ensure_init();
  rust::Vec<CodecSupport> out;
  for (const char* c : {"h264", "h265", "vp9", "av1"}) {
    const char* dec = decoder_for(c);
    bool hw = factory_exists(dec) && is_hw_decoder(dec);
    bool sw = factory_exists(sw_decoder_for(c));
    out.push_back(CodecSupport{rust::String(c), hw, sw});
  }
  return out;
}

std::unique_ptr<Player> gv_create_player(rust::Str codec, uint64_t handle) {
  ensure_init();
  return std::unique_ptr<Player>(avio_create_player(std::string(codec), (guintptr)handle));
}

void gv_start(Player& p) {
  if (p.pipeline) gst_element_set_state(p.pipeline, GST_STATE_PLAYING);
}

bool gv_push_buffer(Player& p, rust::Slice<const uint8_t> data) {
  if (!p.appsrc || data.empty() || p.dead.load()) return false;
  avio_push_player(&p, data.data(), data.size());
  return true;
}

bool gv_is_dead(const Player& p) {
  return p.dead.load();
}

void gv_set_visible(Player& p, bool visible) {
  avio_set_view_hidden(p.view, !visible);
}

void gv_set_content_region(Player& p, double cropL, double cropT, double visW, double visH,
    double tierW, double tierH) {
  if (p.view) avio_set_content_region(p.view, p.sink, cropL, cropT, visW, visH, tierW, tierH);
}

void gv_set_gamma(Player& p, double gamma, double contrast, double gain_r, double gain_g,
    double gain_b) {
  avio_set_gamma_player(&p, gamma, contrast, gain_r, gain_g, gain_b);
}

void gv_stop(Player& p) {
  if (p.pipeline) gst_element_set_state(p.pipeline, GST_STATE_NULL);
  remove_video_view(&p);
}

void gv_set_backdrop(uint64_t handle, double r, double g, double b) {
  if (handle) avio_set_backdrop((guintptr)handle, r, g, b);
}

#ifdef __linux__
// gst-host: runs the pipeline in this separate process with its own GLib main loop. Reads
// create(1)/data(2)/stop(3) frames from the unix socket the main process serves.
struct AvioHost {
  GByteArray* buf;
  GHashTable* players;  // id -> Player*
};

static void avio_free_player(Player* p) {
  delete p;
}

static void avio_host_dispatch(AvioHost* h, guint8 op, guint32 id, const guint8* rest, gsize rlen) {
  gpointer key = GUINT_TO_POINTER(id);
  if (op == 1) {
    char codec[16];
    gsize n = rlen < sizeof(codec) - 1 ? rlen : sizeof(codec) - 1;
    memcpy(codec, rest, n);
    codec[n] = '\0';
    Player* old = (Player*)g_hash_table_lookup(h->players, key);
    if (old) {
      g_hash_table_remove(h->players, key);
      avio_free_player(old);
    }
    Player* p = avio_create_player(codec, 0);
    if (p) {
      gst_element_set_state(p->pipeline, GST_STATE_PLAYING);
      g_hash_table_insert(h->players, key, p);
    }
  } else if (op == 2) {
    Player* p = (Player*)g_hash_table_lookup(h->players, key);
    if (p && p->dead.load()) {
      // The sink (e.g. a bare waylandsink window with no avio-compositor to embed it) is gone:
      // drop this player quietly instead of pushing into a dead pipeline forever. The next
      // create(1) for this id gets a fresh one.
      g_hash_table_remove(h->players, key);
      avio_free_player(p);
      p = nullptr;
    }
    avio_push_player(p, rest, rlen);
  } else if (op == 3) {
    Player* p = (Player*)g_hash_table_lookup(h->players, key);
    if (p) {
      g_hash_table_remove(h->players, key);
      avio_free_player(p);
    }
  } else if (op == 4) {
    Player* p = (Player*)g_hash_table_lookup(h->players, key);
    if (p && rlen >= 5 * sizeof(double)) {
      double v[5];
      memcpy(v, rest, sizeof(v));
      avio_set_gamma_player(p, v[0], v[1], v[2], v[3], v[4]);
    }
  }
}

static gboolean avio_host_readable(gint fd, GIOCondition cond, gpointer data) {
  AvioHost* h = (AvioHost*)data;
  if (cond & (G_IO_HUP | G_IO_ERR)) exit(0);
  guint8 chunk[65536];
  ssize_t n = read(fd, chunk, sizeof(chunk));
  if (n <= 0) exit(0);
  g_byte_array_append(h->buf, chunk, (guint)n);
  while (h->buf->len >= 4) {
    guint32 len;
    memcpy(&len, h->buf->data, 4);
    if (h->buf->len < 4 + len) break;
    if (len >= 5) {
      guint8* payload = h->buf->data + 4;
      guint32 id;
      memcpy(&id, payload + 1, 4);
      avio_host_dispatch(h, payload[0], id, payload + 5, len - 5);
    }
    g_byte_array_remove_range(h->buf, 0, 4 + len);
  }
  return G_SOURCE_CONTINUE;
}

// Where to drop the crash backtrace (next to the AppImage); set in run_host() before the handler arms.
static char g_crash_log_path[1024] = {0};

static void avio_host_crash(int sig) {
  void* frames[64];
  int n = backtrace(frames, 64);
  const char hdr[] = "\n=== gst-host CRASH backtrace ===\n";
  (void)!write(STDERR_FILENO, hdr, sizeof(hdr) - 1);
  backtrace_symbols_fd(frames, n, STDERR_FILENO);
  if (g_crash_log_path[0]) {
    int cf = open(g_crash_log_path, O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (cf >= 0) {
      (void)!write(cf, hdr, sizeof(hdr) - 1);
      backtrace_symbols_fd(frames, n, cf);
      close(cf);
    }
  }
  signal(sig, SIG_DFL);
  raise(sig);
}

// Connect to the host socket and run the GLib main loop. The separate process is the libffi
// fix: outside Electron, libwayland binds the system libffi, not Electron's ABI-incompatible
// bundled copy that corrupts wayland marshalling on resize.
static void avio_host_main(const char* sockPath, const char* crashLogPath) {
  g_set_prgname("avio-video");
  ensure_init();
  if (crashLogPath && crashLogPath[0])
    strncpy(g_crash_log_path, crashLogPath, sizeof(g_crash_log_path) - 1);
  signal(SIGSEGV, avio_host_crash);
  signal(SIGABRT, avio_host_crash);

  int fd = socket(AF_UNIX, SOCK_STREAM, 0);
  struct sockaddr_un addr;
  memset(&addr, 0, sizeof(addr));
  addr.sun_family = AF_UNIX;
  strncpy(addr.sun_path, sockPath, sizeof(addr.sun_path) - 1);
  if (fd < 0 || connect(fd, (struct sockaddr*)&addr, sizeof(addr)) != 0) {
    fprintf(stderr, "[gst-host] connect to %s failed\n", sockPath);
    exit(1);
  }

  AvioHost* h = new AvioHost();
  h->buf = g_byte_array_new();
  h->players = g_hash_table_new(g_direct_hash, g_direct_equal);
  g_unix_fd_add(fd, (GIOCondition)(G_IO_IN | G_IO_HUP | G_IO_ERR), avio_host_readable, h);

  g_main_loop_run(g_main_loop_new(NULL, FALSE));
}

void gv_run_host(rust::Str sock_path, rust::Str crash_path) {
  avio_host_main(std::string(sock_path).c_str(), std::string(crash_path).c_str());
}
#endif
