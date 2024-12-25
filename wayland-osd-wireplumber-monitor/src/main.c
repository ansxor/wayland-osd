#include "lib/log.h"
#include <stdbool.h>
#include <stdio.h>
#include <math.h>
#include <wireplumber-0.5/wp/wp.h>
#include <stdlib.h>
#include <unistd.h>
#include <argp.h>

const char *argp_program_version = "wayland-osd-wireplumber-monitor 1.0";
const char *argp_program_bug_address = "https://github.com/ErikReider/wayland-osd";

static char doc[] = "Wayland OSD Wireplumber Monitor -- A monitor for audio volume changes using wireplumber";
static char args_doc[] = "[CLIENT_PATH]";

static struct argp_option options[] = {
    {"show-device-name", 'd', 0, 0, "Show the audio device name in the OSD", 0},
    {0, 0, 0, 0, 0, 0}
};

typedef struct {
  WpCore *core;
  WpObjectManager *om;
  GPtrArray *apis;
  WpPlugin *mixer_api;
  WpPlugin *def_nodes_api;
  u_int32_t pending_plugins;
  gchar *default_node_name;
  u_int32_t node_id;
  const char *client_path;
  bool show_device_name;
} Context;

struct arguments {
    char *client_path;
    bool show_device_name;
};

static error_t parse_opt(int key, char *arg, struct argp_state *state) {
    struct arguments *arguments = state->input;

    switch (key) {
        case 'd':
            arguments->show_device_name = true;
            break;
        case ARGP_KEY_ARG:
            if (state->arg_num >= 1)
                argp_usage(state);
            arguments->client_path = arg;
            break;
        case ARGP_KEY_END:
            if (state->arg_num == 0)
                arguments->client_path = "wayland-osd-client";
            break;
        default:
            return ARGP_ERR_UNKNOWN;
    }
    return 0;
}

static struct argp argp = {options, parse_opt, args_doc, doc, 0, 0, 0};

bool is_valid_node_id(u_int32_t id) { return id > 0 && id < G_MAXUINT32; }

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

void run_client(const char *client_path, int volume_percent, bool is_muted, const char *device_name) {
  log_debug("Running client with volume: %d%%, muted: %s", volume_percent, is_muted ? "true" : "false");

  pid_t pid = fork();
  if (pid == -1) {
    log_error("Failed to fork process");
    return;
  }

  if (pid == 0) { // Child process
    char volume_str[16];
    snprintf(volume_str, sizeof(volume_str), "%d", volume_percent);

    if (is_muted && device_name != NULL) {
      execl(client_path, client_path, "audio", volume_str, "--mute", "--device", device_name, NULL);
    } else if (is_muted && device_name == NULL) {
      execl(client_path, client_path, "audio", volume_str, "--mute", NULL);
    }  else if (device_name != NULL) {
      execl(client_path, client_path, "audio", volume_str, "--device", device_name, NULL);
    } else {
      execl(client_path, client_path, "audio", volume_str, NULL);
    }
    
    // If execl returns, there was an error
    log_error("Failed to execute client at '%s'", client_path);
    exit(1);
  }
}

void on_update_volume(Context *context, u_int32_t id) {
  log_debug("updating volume", id);
  GVariant *variant = NULL;

  if (!is_valid_node_id(id)) {
    log_error("Invalid node id: %d", id);
    return;
  }

  g_signal_emit_by_name(context->mixer_api, "get-volume", id, &variant);

  if (variant == NULL) {
    log_fatal("Node %d doesn't support volume", id);
    exit(1);
  }

  double raw_volume;
  double raw_min_step;
  bool raw_muted;

  g_variant_lookup(variant, "volume", "d", &raw_volume);
  g_variant_lookup(variant, "step", "d", &raw_min_step);
  g_variant_lookup(variant, "mute", "b", &raw_muted);

  // FIXME: For some reason, trying to free the variant causes a segfault
  // g_clear_pointer(&variant, g_variant_unref);

  int volume = (int)lround(cbrt(raw_volume) * 100);

  log_info("Volume: %d, min_step: %f, muted: %s", volume, raw_min_step, raw_muted ? "true" : "false");
  
  // Call the wayland-osd-client
  
  if (context->show_device_name) {
    log_info("Running client with volume: %d%%, muted: %s, device: %s", volume, raw_muted ? "true" : "false", context->default_node_name);
    run_client(context->client_path, volume, raw_muted, context->default_node_name);
  } else {
    log_info("Running client with volume: %d%%, muted: %s", volume, raw_muted ? "true" : "false");
    run_client(context->client_path, volume, raw_muted, NULL);
  }
}

void on_plugin_activated(__attribute__((unused)) WpObject *p, GAsyncResult *res,
                         Context *context) {
  const gchar *pluginName = wp_plugin_get_name(WP_PLUGIN(p));
  log_info("Plugin activated callback triggered: %s", pluginName);
  g_autoptr(GError) error = NULL;

  if (wp_object_activate_finish(p, res, &error) == 0) {
    log_error("Error activating plugin: %s", error->message);
    exit(1);
    return;
  }

  if (--context->pending_plugins == 0) {
    wp_core_install_object_manager(context->core, context->om);
  }
}

void activate_plugins(Context *context) {
  for (guint i = 0; i < context->apis->len; i++) {
    WpPlugin *plugin = g_ptr_array_index(context->apis, i);
    context->pending_plugins++;
    wp_object_activate(WP_OBJECT(plugin), WP_PLUGIN_FEATURE_ENABLED, NULL,
                       (GAsyncReadyCallback)on_plugin_activated, context);
  }
}

void on_mixer_api_loaded(__attribute__((unused)) WpObject *p, GAsyncResult *res,
                         Context *context) {
  log_info("Mixer API load callback triggered");
  gboolean success = wp_core_load_component_finish(context->core, res, NULL);

  if (success == FALSE) {
    log_fatal("Failed to load mixer api");
    cleanup_context(context);
    exit(1);
  }

  log_info("Mixer API loaded");

  activate_plugins(context);
}

void on_default_nodes_api_loaded(__attribute__((unused)) WpObject *p,
                                 GAsyncResult *res, Context *context) {
  log_info("Default nodes API load callback triggered");
  gboolean success = wp_core_load_component_finish(context->core, res, NULL);

  if (success == FALSE) {
    log_fatal("Failed to load default nodes api");
    cleanup_context(context);
    exit(1);
  }

  log_info("Default nodes API loaded");

  g_ptr_array_add(context->apis,
                  wp_plugin_find(context->core, "default-nodes-api"));
  wp_core_load_component(context->core, "libwireplumber-module-mixer-api",
                         "module", NULL, "mixer-api", NULL,
                         (GAsyncReadyCallback)on_mixer_api_loaded, context);
}

void on_mixer_changed(Context *context, u_int32_t id) {
  log_debug("on_mixer_changed: %d", id);

  g_autoptr(WpNode) node = wp_object_manager_lookup(
      context->om, WP_TYPE_NODE, WP_CONSTRAINT_TYPE_G_PROPERTY, "bound-id",
      "=u", id, NULL);

  if (node == NULL) {
    log_warn("Failed to find node with id %d", id);
    return;
  }

  const gchar *name =
      wp_pipewire_object_get_property(WP_PIPEWIRE_OBJECT(node), "name");

  if (context->node_id != id) {
    log_debug("Ignoring mixed update for node: id: %d, name: %s as it is not "
              "the default node: %s with id: %d",
              id, name, context->default_node_name, context->node_id);
    return;
  }

  on_update_volume(context, id);
}

void on_default_nodes_api_changed(Context *context) {
  log_debug("on_default_nodes_api_changed");

  u_int32_t default_node_id;
  g_signal_emit_by_name(context->def_nodes_api, "get-default-node",
                        "Audio/Sink", &default_node_id);

  if (!is_valid_node_id(default_node_id)) {
    log_warn("Invalid default node id: %d", default_node_id);
    return;
  }

  g_autoptr(WpNode) node = wp_object_manager_lookup(
      context->om, WP_TYPE_NODE, WP_CONSTRAINT_TYPE_G_PROPERTY, "bound-id",
      "=u", default_node_id, NULL);

  if (node == NULL) {
    log_warn("Failed to find node with id %d", default_node_id);
    return;
  }

  const gchar *default_node_name =
      wp_pipewire_object_get_property(WP_PIPEWIRE_OBJECT(node), "node.name");
  
  if (g_strcmp0(default_node_name, context->default_node_name) == 0 && context->node_id == default_node_id) {
    log_debug("Default node name and id match, ignoring");
    return;
  }

  log_debug("Default node changed to %s with id %d", default_node_name, default_node_id);

  g_free(context->default_node_name);
  context->default_node_name = g_strdup(default_node_name);
  context->node_id = default_node_id;
}

void on_object_manager_installed(Context *context) {
  log_debug("Object manager installed");

  context->def_nodes_api = wp_plugin_find(context->core, "default-nodes-api");

  if (context->def_nodes_api == NULL) {
    log_fatal("Default nodes API not loaded");
    cleanup_context(context);
    exit(1);
  }

  context->mixer_api = wp_plugin_find(context->core, "mixer-api");

  if (context->mixer_api == NULL) {
    log_fatal("Mixer API not loaded");
    cleanup_context(context);
    exit(1);
  }

  g_signal_emit_by_name(context->def_nodes_api,
                        "get-default-configured-node-name", "Audio/Sink",
                        &context->default_node_name);
  g_signal_emit_by_name(context->def_nodes_api, "get-default-node",
                        "Audio/Sink", &context->node_id);
  
  g_signal_connect_swapped(context->mixer_api, "changed",
                           G_CALLBACK(on_mixer_changed), context);
  g_signal_connect_swapped(context->def_nodes_api, "changed",
                           G_CALLBACK(on_default_nodes_api_changed), context);
}

bool check_client_executable(const char *client_path) {
  if (access(client_path, F_OK) != 0) {
    log_error("Client not found at '%s'", client_path);
    return false;
  }

  if (access(client_path, X_OK) != 0) {
    log_error("Client at '%s' is not executable", client_path);
    return false;
  }

  return true;
}

int main(int argc, char *argv[]) {
  struct arguments arguments;
  arguments.client_path = NULL;
  arguments.show_device_name = false;

  argp_parse(&argp, argc, argv, 0, 0, &arguments);

  if (!check_client_executable(arguments.client_path)) {
    return 1;
  }

  wp_init(WP_INIT_PIPEWIRE);
  Context *context = g_new0(Context, 1);
  context->core = wp_core_new(NULL, NULL, NULL);
  context->om = wp_object_manager_new();
  context->apis = g_ptr_array_new_with_free_func(g_object_unref);
  context->client_path = arguments.client_path;
  context->show_device_name = arguments.show_device_name;

  log_info("Using client path: %s", arguments.client_path);
  if (arguments.show_device_name) {
    log_info("Device name display enabled");
  }
  log_info("Connecting to pipewire...");

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

  g_signal_connect_swapped(context->om, "installed",
                           G_CALLBACK(on_object_manager_installed), context);

  wp_core_load_component(
      context->core, "libwireplumber-module-default-nodes-api", "module", NULL,
      "default-nodes-api", NULL,
      (GAsyncReadyCallback)on_default_nodes_api_loaded, context);

  // Create and run the main loop
  GMainLoop *loop = g_main_loop_new(NULL, FALSE);
  g_main_loop_run(loop);

  // Cleanup (this will only run after the loop is quit)
  g_main_loop_unref(loop);
  cleanup_context(context);
  return 0;
}
