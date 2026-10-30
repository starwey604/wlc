/* SPDX-License-Identifier: Apache-2.0 */
_Static_assert(_Alignof(demo_rpc_result_value_t) >= _Alignof(response_value_t), "response alignment");
_Static_assert(_Alignof(demo_rpc_result_value_t) >= _Alignof(large_value_t), "large alignment");
_Static_assert(sizeof(demo_rpc_result_value_t) >= sizeof(response_value_t), "response size");
_Static_assert(sizeof(demo_rpc_result_value_t) >= sizeof(large_value_t), "large size");

static void typed_storage_validation(void) {
  static demo_runtime_instance_t instance;
  static union {
    demo_runtime_default_storage_alignment_t alignment;
    uint8_t bytes[4096];
  } arena;
  demo_rpc_result_value_t pool[2];
  demo_runtime_config_t config;
  demo_runtime_requirements_t requirements;
  demo_runtime_init_diagnostic_t diagnostic;
  demo_runtime_storage_t storage = {arena.bytes, sizeof(arena.bytes), pool, 2U};
  assert(demo_runtime_config_defaults(&config) == WL_OK);
  config.rpc_client_enabled = 1;
  config.rpc_server_enabled = 0;
  config.rpc_client_slot_count = 2;
  assert(demo_runtime_requirements(&config, &requirements) == WL_OK);
  assert(requirements.rpc_result_count == 2);
  memset(&instance, 0xAA, sizeof(instance));
  storage.rpc_result_count = 1;
  assert(demo_runtime_init_checked(&instance, &config, &storage, &diagnostic) == WL_ERR_BUF_TOO_SMALL);
  assert(strcmp(diagnostic.field, "storage.rpc_result_count") == 0);
  assert(instance.runtime._reserved == 0xAA);
  storage.rpc_result_count = 2;
  storage.rpc_results = NULL;
  assert(demo_runtime_init_checked(&instance, &config, &storage, &diagnostic) == WL_ERR_INVALID_ARG);
  storage.rpc_results = (demo_rpc_result_value_t *)(void *)((uint8_t *)pool + 1);
  assert(demo_runtime_init_checked(&instance, &config, &storage, &diagnostic) == WL_ERR_INVALID_ARG);
  assert(diagnostic.issue == DEMO_RUNTIME_INIT_STORAGE_ALIGNMENT);
  storage.rpc_results = (demo_rpc_result_value_t *)(void *)arena.bytes;
  assert(demo_runtime_init_checked(&instance, &config, &storage, &diagnostic) == WL_ERR_INVALID_ARG);
  assert(diagnostic.issue == DEMO_RUNTIME_INIT_STORAGE_OVERLAP);
  storage.rpc_results = (demo_rpc_result_value_t *)(void *)&instance;
  assert(demo_runtime_init_checked(&instance, &config, &storage, &diagnostic) == WL_ERR_INVALID_ARG);
  assert(diagnostic.issue == DEMO_RUNTIME_INIT_STORAGE_OVERLAP);
  storage.rpc_results = pool;
  storage.rpc_result_count = SIZE_MAX;
  assert(demo_runtime_init_checked(&instance, &config, &storage, &diagnostic) == WL_ERR_INVALID_ARG);
  assert(diagnostic.issue == DEMO_RUNTIME_INIT_LAYOUT_OVERFLOW);
  storage.rpc_result_count = 2;
  assert(demo_runtime_init_checked(&instance, &config, &storage, &diagnostic) == WL_OK);
  assert(instance.runtime.rpc_results == pool && instance.runtime.rpc_result_count == 2);
}

int main(void) {
  typed_storage_validation();
  demo_rpc_result_value_t pool[2] = {0};
  pool[0].set.has_v = true;
  pool[0].set.v = 1.5;
  pool[1].fetch.has_blob = true;
  pool[1].fetch.blob.length = 256;
  memset(pool[1].fetch.blob.data, 0xAA, 256);
  response_value_t saved = pool[0].set;
  memset(&pool[0], 0, sizeof(pool[0]));
  assert(saved.has_v && saved.v == 1.5);
  assert(pool[1].fetch.blob.data[255] == 0xAA);

  demo_runtime_storage_cursor_t cursor = {NULL, SIZE_MAX, 0};
  const size_t alignment = _Alignof(demo_rpc_result_value_t);
  size_t size = 0;
  assert(demo_runtime_storage_region(&cursor, alignment, 2,
      sizeof(pool[0]), NULL, &size) == WL_OK);
  assert(size == sizeof(pool) && cursor.offset == sizeof(pool));
  size_t before = cursor.offset;
  assert(demo_runtime_storage_region(&cursor, alignment, SIZE_MAX,
      sizeof(pool[0]), NULL, NULL) == WL_ERR_INVALID_ARG);
  assert(cursor.offset == before);
  cursor.offset = SIZE_MAX;
  assert(demo_runtime_storage_region(&cursor, 8, 1,
      sizeof(pool[0]), NULL, NULL) == WL_ERR_INVALID_ARG);
  assert(cursor.offset == SIZE_MAX);
  cursor.offset = SIZE_MAX - 7;
  assert(demo_runtime_storage_region(&cursor, 8, 1, 8,
      NULL, NULL) == WL_ERR_INVALID_ARG);
  assert(cursor.offset == SIZE_MAX - 7);
  cursor.offset = 0;
  cursor.size = sizeof(pool) - 1;
  assert(demo_runtime_storage_region(&cursor, alignment, 2,
      sizeof(pool[0]), NULL, NULL) == WL_ERR_BUF_TOO_SMALL);
  assert(cursor.offset == 0);
  assert(demo_runtime_storage_region(&cursor, 3, 1,
      sizeof(pool[0]), NULL, NULL) == WL_ERR_INVALID_ARG);
  assert(cursor.offset == 0);
  return 0;
}
