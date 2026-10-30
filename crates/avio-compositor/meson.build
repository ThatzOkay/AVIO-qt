project('avio-compositor', 'c',
  default_options: ['c_std=c23,c2x', 'warning_level=1'])

# System wlroots-0.20 when present (Fedora dev box; this default_options list then has no
# effect, it only configures the fallback subproject build below), else the pinned 0.20
# subproject. CI/Ubuntu and the Pi ship older wlroots, so the AppImage carries its own.
#
# backends=drm,libinput + session=enabled so wlr_backend_autocreate() can pick the real
# DRM/KMS backend on bare metal (needed on e.g. Surface RT, which has no host session at
# all) in addition to the nested Wayland backend used on a dev desktop. xwayland stays
# disabled (no X11 apps needed). renderers=gles2/allocators=gbm look GPU-only, but wlroots
# always compiles in the Pixman renderer and the GBM-free drm_dumb allocator regardless of
# these options (see render/meson.build, render/allocator/meson.build) -- wlr_renderer_autocreate()
# and wlr_allocator_autocreate() can already fall back to them at runtime on hardware with no
# working GL driver. wlr_scene and linux-dmabuf stay (always compiled, not gated by any option).
#
# session=enabled needs a seat mechanism at runtime (seatd running, or root + wlroots' direct/
# no-session path) -- confirm which the target rootfs actually has.
wlroots = dependency('wlroots-0.20',
  fallback: ['wlroots', 'wlroots'],
  default_options: [
    'backends=drm,libinput',
    'renderers=gles2',
    'allocators=gbm',
    'session=enabled',
    'xwayland=disabled',
    'color-management=disabled',
    'examples=false',
  ])
wayland_server = dependency('wayland-server')
xkbcommon = dependency('xkbcommon')
pixman = dependency('pixman-1')
cairo = dependency('cairo')
# Direct GLES2 + EGL calls for the output gamma post-process shader.
glesv2 = dependency('glesv2')
egl = dependency('egl')

executable('avio-compositor',
  'avio-compositor.c',
  dependencies: [wlroots, wayland_server, xkbcommon, pixman, cairo, glesv2, egl],
  c_args: ['-DWLR_USE_UNSTABLE', '-D_GNU_SOURCE'],
  install: false)
