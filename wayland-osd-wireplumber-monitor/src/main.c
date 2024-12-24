#include <stdio.h>
#include <wireplumber-0.5/wp/wp.h>
#include "lib/log.h"

typedef struct {
  WpCore *core;
  WpObjectManager *om;
  GPtrArray *apis;
} Context;

static void cleanup_context(Context *context) {
  if (context) {
    if (context->apis)
      g_ptr_array_unref(context->apis);
    if (context->om)
      g_object_unref(context->om);
    if (context->core) {
      wp_core_disconnect(context->core);
      g_object_unref(context->core);
    }
    g_free(context);
  }
}

void onDefaultNodesApiLoaded(__attribute__((unused)) WpObject *p,
                              GAsyncResult *res,
                              Context *context) {
  log_info("Default nodes API load callback triggered");
  gboolean success = wp_core_load_component_finish(context->core, res, NULL);

  if (success == FALSE) {
    log_fatal("Failed to load default nodes api");
    cleanup_context(context);
    exit(1);
  }

  log_info("Default nodes API loaded");
}

int main() {
  wp_init(WP_INIT_PIPEWIRE);
  Context *context = g_new0(Context, 1);
  context->core = wp_core_new(NULL, NULL, NULL);
  context->om = wp_object_manager_new();
  context->apis = g_ptr_array_new_with_free_func(g_object_unref);

  if (!wp_core_connect(context->core)) {
    log_fatal("Failed to connect to PipeWire daemon");
    g_ptr_array_unref(context->apis);
    g_object_unref(context->om);
    g_object_unref(context->core);
    g_free(context);
    return 1;
  }

  log_info("Starting wayland-osd-wireplumber-monitor");
  wp_object_manager_add_interest(context->om, WP_TYPE_NODE,
                                 WP_CONSTRAINT_TYPE_PW_PROPERTY, "media.class",
                                 "=s", "Audio/Sink", NULL);

  wp_core_load_component(context->core,
                         "libwireplumber-module-default-nodes-api", "module",
                         NULL, "default-nodes-api", NULL,
                         (GAsyncReadyCallback)onDefaultNodesApiLoaded, context);

  // Create and run the main loop
  GMainLoop *loop = g_main_loop_new(NULL, FALSE);
  g_main_loop_run(loop);

  // Cleanup (this will only run after the loop is quit)
  g_main_loop_unref(loop);
  cleanup_context(context);
  return 0;
}
