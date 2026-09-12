/* SPDX-License-Identifier: Apache-2.0 */
#include "values.hpp"
#include <cassert>
#include <cmath>
#include <limits>
#include <string>
using namespace device::wlc_native;

int main() {
  assert(static_cast<int>(device::Mode::MODE_ACTIVE) == 1);
  device::ConfigureRequest request{};
  request.settings.enabled = true;
  request.settings.token = {0, 128, 255};
  request.settings.gains = {1.0F, -2.5F, 3.25F};
  request.settings.mode = static_cast<device::Mode>(12345);
  request.settings.class_ = std::string("a\0b", 3);
  request.settings.timeout_ = 123;
  request.opaque = std::vector<std::uint8_t>{}; // present empty != absent
  request.backup.emplace(request.settings);
  request.backup->label = std::string{};
  request.backup->counters = std::array<std::uint64_t, 2>{0, UINT64_MAX};
  assert(!request.settings.label.has_value());
  assert(request.settings.label_or_default() == "设备");
  assert(request.settings.floor_or_default() == INT64_MIN);
  assert(request.settings.ceiling_or_default() == UINT64_MAX);

  configure_request_value_t c{}, decoded{};
  assert(device::wlc_detail::to_c(request, c));
  assert(!c.settings.has_label && c.settings.label.length == 6);
  assert(c.has_opaque && c.opaque.length == 0);
  assert(c.backup.has_label && c.backup.label.length == 0);
  std::uint8_t bytes[1024];
  std::size_t length = 0;
  assert(configure_request_value_encode(&c, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(configure_request_value_decode(bytes, length, &decoded) == WL_CODEC_OK);
  auto response = device::wlc_detail::from_c(decoded);
  request.settings.token.clear();
  c.settings.token.data[0] = 42;
  assert(response.settings.token == std::vector<std::uint8_t>({0, 128, 255}));
  assert(!response.settings.label && response.backup->label->empty());
  assert(response.opaque && response.opaque->empty());
  assert(response.settings.class_ == std::string("a\0b", 3));
  assert(response.settings.timeout_ == 123);
  assert(response.settings.mode == static_cast<device::Mode>(12345));
  assert((*response.backup->counters)[1] == UINT64_MAX);
  assert(response.settings.gains[1] == -2.5F);

  response.settings.label = std::string(13, 'a');
  assert(!device::wlc_detail::to_c(response, c));
  response.settings.label = std::string("\xc0\xaf", 2);
  assert(device::wlc_detail::to_c(response, c));
  assert(configure_request_value_encode(&c, bytes, sizeof(bytes), &length) == WL_CODEC_ERR_UTF8);

  device::Numbers numbers{};
  assert(numbers.offset_or_default() == -7);
  assert(numbers.minimum_or_default() == INT32_MIN);
  assert(numbers.flag_or_default());
  assert(numbers.text_or_default() == "a\"@NAME@");
  numbers.i8 = INT8_MIN; numbers.u8 = UINT8_MAX;
  numbers.i16 = INT16_MIN; numbers.u16 = UINT16_MAX;
  numbers.i32 = INT32_MIN; numbers.u32 = UINT32_MAX;
  numbers.i64 = INT64_MIN; numbers.u64 = UINT64_MAX;
  numbers.f32 = UINT32_MAX; numbers.f64 = UINT64_MAX;
  numbers.single = -0.0F; numbers.double_ = std::numeric_limits<double>::infinity();
  numbers.doubles = std::array<double, 2>{1.5, -0.0};
  numbers.words = {0, UINT32_MAX};
  numbers_value_t nc{}, nd{};
  assert(device::wlc_detail::to_c(numbers, nc));
  assert(numbers_value_encode(&nc, bytes, sizeof(bytes), &length) == WL_CODEC_OK);
  assert(numbers_value_decode(bytes, length, &nd) == WL_CODEC_OK);
  const auto result = device::wlc_detail::from_c(nd);
  assert(result.i8 == INT8_MIN && result.u8 == UINT8_MAX);
  assert(result.i16 == INT16_MIN && result.u16 == UINT16_MAX);
  assert(result.i32 == INT32_MIN && result.u32 == UINT32_MAX);
  assert(result.i64 == INT64_MIN && result.u64 == UINT64_MAX);
  assert(result.f32 == UINT32_MAX && result.f64 == UINT64_MAX);
  assert(std::signbit(result.single) && std::isinf(result.double_));
  assert((*result.doubles)[0] == 1.5 && std::signbit((*result.doubles)[1]));
  assert(result.words[1] == UINT32_MAX);
}
