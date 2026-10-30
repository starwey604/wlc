/* SPDX-License-Identifier: Apache-2.0 */
#include "mixed.c"
#include <assert.h>

_Static_assert(CHILD_CODEC_PROFILE == (POSITIONAL ? 3U : 1U), "Child route codec");
_Static_assert(PARENT_CODEC_PROFILE == 1U, "Parent remains Schema-v1");
_Static_assert(GRANDPARENT_CODEC_PROFILE == 1U, "Transitive Schema-v1 dependency");
_Static_assert(CHILD_MAX_ENCODED_SIZE == (PACKED ? (POSITIONAL ? 14U : 15U) : 5U), "Child bound");
_Static_assert(PARENT_MAX_ENCODED_SIZE == (PACKED ? 17U : 7U), "Parent nested bound");
_Static_assert(GRANDPARENT_MAX_ENCODED_SIZE == (PACKED ? 19U : 9U), "Transitive bound");

static uint64_t fingerprint(const uint8_t *bytes, size_t length) {
  uint64_t hash = UINT64_C(0xcbf29ce484222325);
  for (size_t i = 0U; i < length; ++i) hash = (hash ^ bytes[i]) * UINT64_C(0x100000001b3);
  return hash;
}

int main(void) {
  const uint8_t tagged_child[] = {0x0d, 0x12, 0x34, 0x56, 0x78,
#if PACKED
      0x12, 0x08, 0x00, 0x00, 0x00, 0x01, 0xff, 0xff, 0xff, 0xff,
#endif
  };
  const uint8_t positional_child[] = {0xf8, 0xac, 0xd1, 0x91, 0x01,
#if PACKED
      0x08, 0x00, 0x00, 0x00, 0x01, 0xff, 0xff, 0xff, 0xff,
#endif
  };
  uint8_t expected_parent[sizeof(tagged_child) + 2U];
  uint8_t expected_grandparent[sizeof(expected_parent) + 2U];
  expected_parent[0] = expected_grandparent[0] = 0x0a;
  expected_parent[1] = (uint8_t)sizeof(tagged_child);
  expected_grandparent[1] = (uint8_t)sizeof(expected_parent);
  memcpy(expected_parent + 2U, tagged_child, sizeof(tagged_child));
  memcpy(expected_grandparent + 2U, expected_parent, sizeof(expected_parent));
  const uint8_t *expected_child = POSITIONAL ? positional_child : tagged_child;
  const size_t child_length = POSITIONAL ? sizeof(positional_child) : sizeof(tagged_child);
  child_t child = {.has_value = true, .value = UINT32_C(0x12345678)};
#if PACKED
  child.has_words = true;
  child.words[0] = 1U; child.words[1] = UINT32_MAX;
#endif
  parent_t parent = {.has_child = true, .child = child}, decoded;
  grandparent_t grandparent = {.has_parent = true, .parent = parent}, grand_decoded;
  uint8_t bytes[64]; size_t length;
  assert(child_encoded_size(&child) == child_length);
  assert(child_encode(&child, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == child_length && memcmp(bytes, expected_child, length) == 0);
  child_t child_decoded;
  assert(child_decode(expected_child, child_length, &child_decoded) == WL_CODEC_OK);
  assert(child_decoded.has_value && child_decoded.value == child.value);
  assert(parent_encoded_size(&parent) == sizeof(expected_parent));
  assert(parent_encode(&parent, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(expected_parent) && memcmp(bytes, expected_parent, length) == 0);
  assert(parent_decode(expected_parent, sizeof(expected_parent), &decoded) == WL_CODEC_OK);
  assert(decoded.has_child && decoded.child.has_value && decoded.child.value == child.value);
  assert(grandparent_encode(&grandparent, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(expected_grandparent) && memcmp(bytes, expected_grandparent, length) == 0);
  assert(grandparent_decode(expected_grandparent, sizeof(expected_grandparent), &grand_decoded) == WL_CODEC_OK);
  assert(grand_decoded.parent.child.value == child.value);

  child_value_t owned_child;
  parent_value_t owned_parent;
  grandparent_value_t owned_grandparent;
  assert(child_value_decode(expected_child, child_length, &owned_child) == WL_CODEC_OK);
  assert(parent_value_decode(expected_parent, sizeof(expected_parent), &owned_parent) == WL_CODEC_OK);
  assert(grandparent_value_decode(expected_grandparent, sizeof(expected_grandparent), &owned_grandparent) == WL_CODEC_OK);
  assert(owned_child.value == child.value && owned_parent.child.value == child.value);
  assert(owned_grandparent.parent.child.value == child.value);
#if PACKED
  assert(owned_parent.child.has_words && owned_parent.child.words[1] == UINT32_MAX);
#endif
  assert(child_value_encode(&owned_child, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == child_length && memcmp(bytes, expected_child, length) == 0);
  assert(parent_value_encoded_size(&owned_parent) == sizeof(expected_parent));
  assert(parent_value_encode(&owned_parent, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(expected_parent) && memcmp(bytes, expected_parent, length) == 0);
  assert(grandparent_value_encode(&owned_grandparent, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(length == sizeof(expected_grandparent) && memcmp(bytes, expected_grandparent, length) == 0);
  assert(parent_value_to_view(&owned_parent, &decoded) == WL_CODEC_OK);
  assert(parent_value_from_view(&decoded, &owned_parent) == WL_CODEC_OK);
  uint64_t hash = UINT64_C(0xcbf29ce484222325);
#if !POSITIONAL
  assert(parent_wlc_detail_fingerprint(&decoded, &hash, &length) == WL_CODEC_OK);
  assert(length == sizeof(expected_parent) && hash == fingerprint(expected_parent, length));
  hash = UINT64_C(0xcbf29ce484222325);
#endif
  assert(grandparent_wlc_detail_fingerprint(&grand_decoded, &hash, &length) == WL_CODEC_OK);
  assert(length == sizeof(expected_grandparent) && hash == fingerprint(expected_grandparent, length));
  // Failed owned decode must preserve the complete output, including nesting.
  parent_value_t before;
  memcpy(&before, &owned_parent, sizeof(before));
  assert(parent_value_decode(expected_parent, sizeof(expected_parent) - 1U, &owned_parent) != WL_CODEC_OK);
  assert(memcmp(&before, &owned_parent, sizeof(before)) == 0);
  return 0;
}
