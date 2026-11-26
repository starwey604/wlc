/* SPDX-License-Identifier: Apache-2.0 */
#include "demo.h"
#include <assert.h>
#include <stddef.h>

static_assert(DEMO_REQUEST_FRAME_MAX == 2014 && DEMO_REPLY_FRAME_MAX == 18 &&
              DEMO_MESSAGE_FRAME_MAX == 6, "storage padding is not payload");
static_assert(DEMO_RX_FRAME_MAX == 2014 && DEMO_TX_FRAME_MAX == 2014 &&
              DEMO_RX_WIRE_MAX == 2014 && DEMO_TX_WIRE_MAX == 2014, "native wire limits");

static_assert(alignof(demo_node_t) >= WL_NODE_BUFFER_ALIGN, "node base");
static_assert(offsetof(demo_node_t, requests) % WL_NODE_BUFFER_ALIGN == 0, "request pool base");
static_assert(offsetof(demo_node_t, replies) % WL_NODE_BUFFER_ALIGN == 0, "reply pool base");
static_assert(offsetof(demo_node_t, message) % WL_NODE_BUFFER_ALIGN == 0, "message base");
static_assert(offsetof(demo_node_t, rx) % WL_NODE_BUFFER_ALIGN == 0, "RX pool base");
static_assert(sizeof(((demo_node_t *)0)->requests[0]) == 2016, "request slot stride");
static_assert(sizeof(((demo_node_t *)0)->replies[0]) == 20, "reply slot stride");
static_assert(DEMO_RX_UNIT_SIZE == 2048U, "RX stride rounded up to 512 bytes");
static_assert(sizeof(((demo_node_t *)0)->rx[0]) == DEMO_RX_UNIT_SIZE, "RX slot stride");
