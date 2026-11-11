/* SPDX-License-Identifier: Apache-2.0 */
/* Frozen original-A reader and tagged Chunk consumers, before early return. */
static wl_codec_status_t reference_getv(const uint8_t *in, size_t length, size_t *at,
                                  uint64_t *out) {
  size_t start = *at, n = 0U;
  uint64_t v = 0U;
  while (*at < length && n < 10U) {
    uint8_t b = in[(*at)++];
    if (n == 9U && b > 1U) return WL_CODEC_ERR_OVERFLOW;
    v |= (uint64_t)(b & 127U) << (7U * n++);
    if ((b & 128U) == 0U) {
      if (wlc_vsize(v) != *at - start) return WL_CODEC_ERR_MALFORMED;
      *out = v;
      return WL_CODEC_OK;
    }
  }
  return *at == length ? WL_CODEC_ERR_MALFORMED : WL_CODEC_ERR_OVERFLOW;
}
#ifndef READER_ONLY
static wl_codec_status_t reference_skip(uint8_t wire, const uint8_t *in, size_t n,
                                  size_t *at) {
  uint64_t length;
  wl_codec_status_t s;
  if (wire == 0U) return reference_getv(in, n, at, &length);
  if (wire == 1U) {
    if (n - *at < 8U) return WL_CODEC_ERR_MALFORMED;
    *at += 8U;
    return WL_CODEC_OK;
  }
  if (wire == 5U) {
    if (n - *at < 4U) return WL_CODEC_ERR_MALFORMED;
    *at += 4U;
    return WL_CODEC_OK;
  }
  if (wire != 2U) return WL_CODEC_ERR_MALFORMED;
  if ((s = reference_getv(in, n, at, &length)) != WL_CODEC_OK) return s;
  if (length > n - *at) return WL_CODEC_ERR_MALFORMED;
  *at += (size_t)length;
  return WL_CODEC_OK;
}

static wl_codec_status_t reference_read_value(const wlc_field_t *f, const uint8_t *in,
                                        size_t n, size_t *at, void *out) {
  (void)in; (void)n; (void)at; (void)out;
  switch (f->kind) {
    case WLC_F32:
    case WLC_F64:
      return wlc_read_fixed(f->kind, in, n, at, out);
    case WLC_BYTES:
      {
      uint64_t v;
      wl_codec_status_t s = reference_getv(in, n, at, &v);
      if (s != WL_CODEC_OK) return s;
      if ((f->kind == WLC_BYTES || f->kind == WLC_STRING) &&
          f->max_length != 0U && v > (uint64_t)f->max_length)
        return WL_CODEC_ERR_INVALID_VALUE;
      if (v > n - *at) return WL_CODEC_ERR_MALFORMED;
      size_t bytes = (size_t)v;
      if (f->kind == WLC_BYTES)
        *(wl_codec_bytes_t *)out = (wl_codec_bytes_t){in + *at, bytes};
      *at += bytes;
      return WL_CODEC_OK;
    }
    default: break;
  }
  return WL_CODEC_ERR_INVALID_VALUE;
}

static inline wl_codec_status_t reference_decode(const wlc_desc_t *d, const uint8_t *in,
                                    size_t n, void *out) {
  if (d == NULL || out == NULL || (n != 0U && in == NULL))
    return WL_CODEC_ERR_INVALID_VALUE;
  wlc_clear(d, out);
  // A hint only: missing, unknown or reordered fields still use exact lookup.
  // Each recursive decode owns its cursor; empty schemas avoid NULL arithmetic.
  const wlc_field_t *next = d->fields;
  const wlc_field_t *end = d->count == 0U ? next : next + d->count;
  for (size_t at = 0U; at < n;) {
    uint64_t key;
    wl_codec_status_t s = reference_getv(in, n, &at, &key);
    if (s != WL_CODEC_OK) return s;
    uint64_t raw_number = key >> 3U;
    uint8_t wire = (uint8_t)(key & 7U);
    if (raw_number == 0U || raw_number > 65535U ||
        (wire != 0U && wire != 1U && wire != 2U && wire != 5U))
      return WL_CODEC_ERR_MALFORMED;
    uint16_t number = (uint16_t)raw_number;
    const wlc_field_t *f = next != end && next->number == number
                               ? next : wlc_find_field(d, number);
    if (f == NULL) {
      if ((s = reference_skip(wire, in, n, &at)) != WL_CODEC_OK) return s;
      continue;
    }
    if (wire != wlc_wire(f)) return WL_CODEC_ERR_WIRE_TYPE;
    next = f->card == WLC_REPEATED ? f : f + 1;
    uint8_t *base = out;
    void *p = base + f->value;
    if (f->card == WLC_OPTIONAL) {
      if (*(bool *)(base + f->has)) return WL_CODEC_ERR_DUPLICATE_FIELD;
    }
    if ((s = reference_read_value(f, in, n, &at, p)) != WL_CODEC_OK) return s;
    if (f->card == WLC_OPTIONAL) *(bool *)(base + f->has) = true;
  }
  for (size_t i = 0U; i < d->count; ++i) {
    const wlc_field_t *f = &d->fields[i];
    if (f->required != 0U && !*(const bool *)((uint8_t *)out + f->has))
      return WL_CODEC_ERR_MISSING_REQUIRED_FIELD;
  }
  return WL_CODEC_OK;
}
#endif
