/*
 * test_pack.c - Tests for yar_pack (msgpack serialization wrapper)
 *
 * yar_pack wraps msgpack-c. The serialized format is standard msgpack.
 *
 * Type mapping:
 *   YAR_DATA_NULL   (1) -> msgpack nil        (0xc0)
 *   YAR_DATA_BOOL   (2) -> msgpack bool       (0xc2=false, 0xc3=true)
 *   YAR_DATA_LONG   (3) -> msgpack int64      (0xd3 + 8 bytes, big-endian)
 *   YAR_DATA_ULONG  (4) -> msgpack uint64     (0xcf + 8 bytes, big-endian)
 *   YAR_DATA_DOUBLE (5) -> msgpack double     (0xcb + 8 bytes, IEEE 754 big-endian)
 *   YAR_DATA_STRING (6) -> msgpack str        (fixstr/str8/str16/str32 + bytes)
 *   YAR_DATA_MAP    (7) -> msgpack map        (fixmap/map16/map32 + kv pairs)
 *   YAR_DATA_ARRAY  (8) -> msgpack array      (fixarray/array16/array32 + elements)
 *
 * IMPORTANT for Rust rewrite:
 *   - yar_pack_push_long ALWAYS packs as int64 (msgpack 0xd3), even for small values
 *   - yar_pack_push_ulong ALWAYS packs as uint64 (msgpack 0xcf), even for small values
 *   - yar_pack_push_double ALWAYS packs as 64-bit double (msgpack 0xcb)
 *   - Map iterator size = map_entries * 2 (interleaved key, value, key, value...)
 *   - yar_pack_push_packager copies raw sbuffer bytes (appends one msgpack object)
 *   - yar_pack_start_null() creates an empty packager; pushing null then data
 *     creates two adjacent msgpack objects in the buffer
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <math.h>
#include <stdint.h>
#include <inttypes.h>
#include "yar.h"

static int tests_run = 0;
static int tests_passed = 0;

#define TEST_ASSERT(cond, msg) do { \
    tests_run++; \
    if (!(cond)) { \
        printf("  FAIL: %s (line %d)\n", msg, __LINE__); \
    } else { \
        tests_passed++; \
        printf("  PASS: %s\n", msg); \
    } \
} while(0)

static void hex_dump(const char *label, const unsigned char *data, size_t len) {
    size_t i;
    printf("  %s (%zu bytes): ", label, len);
    for (i = 0; i < len && i < 128; i++) {
        printf("%02x", data[i]);
    }
    if (len > 128) printf("...");
    printf("\n");
}

/* ---- Null ---- */

static void test_pack_null(void) {
    yar_packager *pk;
    yar_payload payload = {0};
    yar_unpackager *unpk;
    const yar_data *data;
    uint size = 0;

    printf("\n[test_pack_null]\n");

    pk = yar_pack_start_null();
    yar_pack_push_null(pk);
    yar_pack_to_string(pk, &payload);

    hex_dump("null msgpack", (unsigned char *)payload.data, payload.size);
    TEST_ASSERT(payload.size == 1, "null serializes to 1 byte");
    TEST_ASSERT((unsigned char)payload.data[0] == 0xc0, "null byte is 0xc0");

    unpk = yar_unpack_init(payload.data, payload.size);
    TEST_ASSERT(unpk != NULL, "unpack_init succeeds");

    data = yar_unpack_unpack(unpk);
    TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_NULL, "type is YAR_DATA_NULL");

    yar_unpack_free(unpk);
    yar_pack_free(pk);
}

/* ---- Bool ---- */

static void test_pack_bool_true(void) {
    yar_packager *pk;
    yar_payload payload = {0};
    yar_unpackager *unpk;
    const yar_data *data;
    uint size = 0;
    int val = 0;

    printf("\n[test_pack_bool_true]\n");

    pk = yar_pack_start_bool();
    yar_pack_push_bool(pk, 1);
    yar_pack_to_string(pk, &payload);

    hex_dump("bool true msgpack", (unsigned char *)payload.data, payload.size);
    TEST_ASSERT(payload.size == 1, "bool true serializes to 1 byte");
    TEST_ASSERT((unsigned char)payload.data[0] == 0xc3, "true byte is 0xc3");

    unpk = yar_unpack_init(payload.data, payload.size);
    data = yar_unpack_unpack(unpk);
    TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_BOOL, "type is YAR_DATA_BOOL");
    yar_unpack_data_bool(data, &val);
    TEST_ASSERT(val == 1, "value is 1 (true)");

    yar_unpack_free(unpk);
    yar_pack_free(pk);
}

static void test_pack_bool_false(void) {
    yar_packager *pk;
    yar_payload payload = {0};
    yar_unpackager *unpk;
    const yar_data *data;
    uint size = 0;
    int val = 1;

    printf("\n[test_pack_bool_false]\n");

    pk = yar_pack_start_bool();
    yar_pack_push_bool(pk, 0);
    yar_pack_to_string(pk, &payload);

    hex_dump("bool false msgpack", (unsigned char *)payload.data, payload.size);
    TEST_ASSERT(payload.size == 1, "bool false serializes to 1 byte");
    TEST_ASSERT((unsigned char)payload.data[0] == 0xc2, "false byte is 0xc2");

    unpk = yar_unpack_init(payload.data, payload.size);
    data = yar_unpack_unpack(unpk);
    TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_BOOL, "type is YAR_DATA_BOOL");
    yar_unpack_data_bool(data, &val);
    TEST_ASSERT(val == 0, "value is 0 (false)");

    yar_unpack_free(unpk);
    yar_pack_free(pk);
}

/* ---- Long (signed int64) ---- */

static void test_pack_long(void) {
    long test_values[] = {0, -1, 42, -128, 127, INT64_MIN, INT64_MAX};
    int num_tests = sizeof(test_values) / sizeof(test_values[0]);
    int i;

    printf("\n[test_pack_long]\n");

    for (i = 0; i < num_tests; i++) {
        yar_packager *pk;
        yar_payload payload = {0};
        yar_unpackager *unpk;
        const yar_data *data;
        uint size = 0;
        long val = 0;

        pk = yar_pack_start_long();
        yar_pack_push_long(pk, test_values[i]);
        yar_pack_to_string(pk, &payload);

        if (i < 3) {
            char label[64];
            snprintf(label, sizeof(label), "long %ld msgpack", test_values[i]);
            hex_dump(label, (unsigned char *)payload.data, payload.size);
        }

        /*
         * NOTE: msgpack_pack_int64 uses compact encoding for small values:
         *   -128..-33  -> 2 bytes (int8:  0xd0 + 1 byte)
         *   -32..-1    -> 1 byte  (negative fixint: 0xe0..0xff)
         *    0..127    -> 1 byte  (positive fixint: 0x00..0x7f)
         *    128..255  -> 2 bytes (uint8:  0xcc + 1 byte)
         *    256..65535-> 3 bytes (uint16: 0xcd + 2 bytes)
         *    ...
         *    large vals-> 9 bytes (int64:  0xd3 + 8 bytes)
         * Only values outside the compact range produce 9-byte int64 encoding.
         */
        TEST_ASSERT(payload.size >= 1 && payload.size <= 9,
            "long serializes to 1-9 bytes (msgpack compact encoding)");

        unpk = yar_unpack_init(payload.data, payload.size);
        TEST_ASSERT(unpk != NULL, "unpack_init succeeds");

        data = yar_unpack_unpack(unpk);
        yar_data_type dtype = yar_unpack_data_type(data, &size);

        /* msgpack-c decodes: value >= 0 -> POSITIVE_INTEGER (ULONG),
         * value < 0 -> NEGATIVE_INTEGER (LONG).
         * Zero packed as int64 decodes as positive fixint -> ULONG. */
        if (test_values[i] >= 0) {
            TEST_ASSERT(dtype == YAR_DATA_ULONG, "non-negative long decodes as ULONG");
            ulong uval = 0;
            yar_unpack_data_ulong(data, &uval);
            TEST_ASSERT((long)uval == test_values[i], "ulong value matches original");
        } else {
            TEST_ASSERT(dtype == YAR_DATA_LONG, "negative long decodes as LONG");
            yar_unpack_data_long(data, &val);
            TEST_ASSERT(val == test_values[i], "long value matches original");
        }

        yar_unpack_free(unpk);
        yar_pack_free(pk);
    }

    /* Print test vector for long value 42 */
    {
        yar_packager *pk = yar_pack_start_long();
        yar_payload payload = {0};
        yar_pack_push_long(pk, 42);
        yar_pack_to_string(pk, &payload);
        hex_dump("TEST VECTOR long(42)", (unsigned char *)payload.data, payload.size);
        yar_pack_free(pk);
    }
}

/* ---- Ulong (unsigned int64) ---- */

static void test_pack_ulong(void) {
    ulong test_values[] = {0, 1, 255, 65535, UINT64_MAX};
    int num_tests = sizeof(test_values) / sizeof(test_values[0]);
    int i;

    printf("\n[test_pack_ulong]\n");

    for (i = 0; i < num_tests; i++) {
        yar_packager *pk;
        yar_payload payload = {0};
        yar_unpackager *unpk;
        const yar_data *data;
        uint size = 0;
        ulong val = 0;

        pk = yar_pack_start_ulong();
        yar_pack_push_ulong(pk, test_values[i]);
        yar_pack_to_string(pk, &payload);

        if (i < 3) {
            char label[64];
            snprintf(label, sizeof(label), "ulong %" PRIu64 " msgpack", (uint64_t)test_values[i]);
            hex_dump(label, (unsigned char *)payload.data, payload.size);
        }

        /*
         * NOTE: msgpack_pack_uint64 uses compact encoding for small values:
         *    0..127    -> 1 byte  (positive fixint)
         *    128..255  -> 2 bytes (uint8:  0xcc + 1 byte)
         *    256..65535-> 3 bytes (uint16: 0xcd + 2 bytes)
         *    ...
         *    large vals-> 9 bytes (uint64: 0xcf + 8 bytes)
         */
        TEST_ASSERT(payload.size >= 1 && payload.size <= 9,
            "ulong serializes to 1-9 bytes (msgpack compact encoding)");

        unpk = yar_unpack_init(payload.data, payload.size);
        data = yar_unpack_unpack(unpk);
        TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_ULONG, "type is YAR_DATA_ULONG");
        yar_unpack_data_ulong(data, &val);
        TEST_ASSERT(val == test_values[i], "ulong value matches original");

        yar_unpack_free(unpk);
        yar_pack_free(pk);
    }

    /* Print test vector for ulong 1000 */
    {
        yar_packager *pk = yar_pack_start_ulong();
        yar_payload payload = {0};
        yar_pack_push_ulong(pk, 1000);
        yar_pack_to_string(pk, &payload);
        hex_dump("TEST VECTOR ulong(1000)", (unsigned char *)payload.data, payload.size);
        yar_pack_free(pk);
    }
}

/* ---- Double ---- */

static void test_pack_double(void) {
    double test_values[] = {0.0, 3.14, -1.5, 1e100, -1e-100};
    int num_tests = sizeof(test_values) / sizeof(test_values[0]);
    int i;

    printf("\n[test_pack_double]\n");

    for (i = 0; i < num_tests; i++) {
        yar_packager *pk;
        yar_payload payload = {0};
        yar_unpackager *unpk;
        const yar_data *data;
        uint size = 0;
        double val = 0;

        pk = yar_pack_start_double();
        yar_pack_push_double(pk, test_values[i]);
        yar_pack_to_string(pk, &payload);

        if (i < 3) {
            char label[64];
            snprintf(label, sizeof(label), "double %g msgpack", test_values[i]);
            hex_dump(label, (unsigned char *)payload.data, payload.size);
        }

        /* double always packs as 0xcb + 8 bytes = 9 bytes */
        TEST_ASSERT(payload.size == 9, "double serializes to 9 bytes");
        TEST_ASSERT((unsigned char)payload.data[0] == 0xcb, "double prefix is 0xcb");

        unpk = yar_unpack_init(payload.data, payload.size);
        data = yar_unpack_unpack(unpk);
        TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_DOUBLE, "type is YAR_DATA_DOUBLE");
        yar_unpack_data_value(data, &val);
        TEST_ASSERT(fabs(val - test_values[i]) < 1e-15 || (test_values[i] == 0.0 && val == 0.0),
            "double value matches original");

        yar_unpack_free(unpk);
        yar_pack_free(pk);
    }

    /* Test vector for double 3.14 */
    {
        yar_packager *pk = yar_pack_start_double();
        yar_payload payload = {0};
        yar_pack_push_double(pk, 3.14);
        yar_pack_to_string(pk, &payload);
        hex_dump("TEST VECTOR double(3.14)", (unsigned char *)payload.data, payload.size);
        yar_pack_free(pk);
    }
}

/* ---- String ---- */

static void test_pack_string(void) {
    printf("\n[test_pack_string]\n");

    /* Empty string */
    {
        yar_packager *pk = yar_pack_start_string();
        yar_payload payload = {0};
        yar_unpackager *unpk;
        const yar_data *data;
        uint size = 0;
        const char *str = NULL;

        yar_pack_push_string(pk, "", 0);
        yar_pack_to_string(pk, &payload);
        hex_dump("empty string msgpack", (unsigned char *)payload.data, payload.size);

        /* fixstr with length 0: 0xa0 */
        TEST_ASSERT(payload.size == 1, "empty string is 1 byte (fixstr len=0)");
        TEST_ASSERT((unsigned char)payload.data[0] == 0xa0, "empty string byte is 0xa0");

        unpk = yar_unpack_init(payload.data, payload.size);
        data = yar_unpack_unpack(unpk);
        TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_STRING, "type is STRING");
        TEST_ASSERT(size == 0, "string length is 0");
        yar_unpack_data_string(data, &str);
        /* Note: msgpack-c may return NULL for zero-length strings */
        TEST_ASSERT(size == 0, "empty string length is 0 after unpack");

        yar_unpack_free(unpk);
        yar_pack_free(pk);
    }

    /* "hello" */
    {
        yar_packager *pk = yar_pack_start_string();
        yar_payload payload = {0};
        yar_unpackager *unpk;
        const yar_data *data;
        uint size = 0;
        const char *str = NULL;

        yar_pack_push_string(pk, "hello", 5);
        yar_pack_to_string(pk, &payload);
        hex_dump("'hello' msgpack", (unsigned char *)payload.data, payload.size);

        /* fixstr with length 5: 0xa5 + "hello" = 6 bytes */
        TEST_ASSERT(payload.size == 6, "'hello' is 6 bytes (fixstr)");
        TEST_ASSERT((unsigned char)payload.data[0] == 0xa5, "fixstr header is 0xa5");
        TEST_ASSERT(memcmp(payload.data + 1, "hello", 5) == 0, "string body is 'hello'");

        unpk = yar_unpack_init(payload.data, payload.size);
        data = yar_unpack_unpack(unpk);
        TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_STRING, "type is STRING");
        TEST_ASSERT(size == 5, "string length is 5");
        yar_unpack_data_string(data, &str);
        TEST_ASSERT(memcmp(str, "hello", 5) == 0, "unpacked string is 'hello'");

        yar_unpack_free(unpk);
        yar_pack_free(pk);
    }

    /* Binary data (string with embedded nulls) */
    {
        yar_packager *pk = yar_pack_start_string();
        yar_payload payload = {0};
        yar_unpackager *unpk;
        const yar_data *data;
        uint size = 0;
        const char *str = NULL;
        char binary[] = {0x00, 0x01, 0x02, 0xff, 0x00};

        yar_pack_push_string(pk, binary, 5);
        yar_pack_to_string(pk, &payload);
        hex_dump("binary string msgpack", (unsigned char *)payload.data, payload.size);

        unpk = yar_unpack_init(payload.data, payload.size);
        data = yar_unpack_unpack(unpk);
        TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_STRING, "binary data type is STRING");
        TEST_ASSERT(size == 5, "binary string length is 5");
        yar_unpack_data_string(data, &str);
        TEST_ASSERT(memcmp(str, binary, 5) == 0, "binary string roundtrip matches");

        yar_unpack_free(unpk);
        yar_pack_free(pk);
    }

    /* Test vector for string "hello" */
    {
        yar_packager *pk = yar_pack_start_string();
        yar_payload payload = {0};
        yar_pack_push_string(pk, "hello", 5);
        yar_pack_to_string(pk, &payload);
        hex_dump("TEST VECTOR string('hello')", (unsigned char *)payload.data, payload.size);
        yar_pack_free(pk);
    }
}

/* ---- Array ---- */

static void test_pack_array(void) {
    yar_packager *pk;
    yar_payload payload = {0};
    yar_unpackager *unpk;
    const yar_data *data;
    uint size = 0;

    printf("\n[test_pack_array]\n");

    /* Array of 3 elements: null, bool(true), long(42) */
    pk = yar_pack_start_array(3);
    yar_pack_push_null(pk);
    yar_pack_push_bool(pk, 1);
    yar_pack_push_long(pk, 42);
    yar_pack_to_string(pk, &payload);

    hex_dump("array[null,true,42] msgpack", (unsigned char *)payload.data, payload.size);

    /* Expected: fixarray(3)=0x93, nil=0xc0, true=0xc3, fixint(42)=0x2a = 4 bytes.
     * Note: msgpack_pack_int64(42) uses compact fixint encoding, not full int64. */
    TEST_ASSERT(payload.size == 4, "array[null,true,42] is 4 bytes (compact encoding)");
    TEST_ASSERT((unsigned char)payload.data[0] == 0x93, "fixarray(3) header is 0x93");

    unpk = yar_unpack_init(payload.data, payload.size);
    data = yar_unpack_unpack(unpk);
    TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_ARRAY, "type is ARRAY");
    TEST_ASSERT(size == 3, "array size is 3");

    /* Iterate through array elements */
    {
        yar_unpack_iterator *it = yar_unpack_iterator_init(data);
        const yar_data *elem;
        uint esize;

        TEST_ASSERT(it != NULL, "iterator created");

        /* Element 0: null */
        elem = yar_unpack_iterator_current(it);
        TEST_ASSERT(yar_unpack_data_type(elem, &esize) == YAR_DATA_NULL, "elem[0] is NULL");

        /* Element 1: bool true */
        TEST_ASSERT(yar_unpack_iterator_next(it) != 0, "iterator next to elem[1]");
        elem = yar_unpack_iterator_current(it);
        TEST_ASSERT(yar_unpack_data_type(elem, &esize) == YAR_DATA_BOOL, "elem[1] is BOOL");
        int bval = 0;
        yar_unpack_data_bool(elem, &bval);
        TEST_ASSERT(bval == 1, "elem[1] value is true");

        /* Element 2: long 42 (decoded as ULONG since >= 0) */
        TEST_ASSERT(yar_unpack_iterator_next(it) != 0, "iterator next to elem[2]");
        elem = yar_unpack_iterator_current(it);
        yar_data_type t = yar_unpack_data_type(elem, &esize);
        TEST_ASSERT(t == YAR_DATA_ULONG || t == YAR_DATA_LONG,
            "elem[2] is LONG or ULONG");
        if (t == YAR_DATA_ULONG) {
            ulong lval = 0;
            yar_unpack_data_ulong(elem, &lval);
            TEST_ASSERT(lval == 42, "elem[2] value is 42");
        } else {
            long lval = 0;
            yar_unpack_data_long(elem, &lval);
            TEST_ASSERT(lval == 42, "elem[2] value is 42");
        }

        /* No more elements */
        TEST_ASSERT(yar_unpack_iterator_next(it) == 0, "no more elements after elem[2]");

        yar_unpack_iterator_free(it);
    }

    yar_unpack_free(unpk);
    yar_pack_free(pk);
}

/* ---- Map ---- */

static void test_pack_map(void) {
    yar_packager *pk;
    yar_payload payload = {0};
    yar_unpackager *unpk;
    const yar_data *data;
    uint size = 0;

    printf("\n[test_pack_map]\n");

    /* Map of 2 entries: {"name" => "yar", "version" => 1} */
    pk = yar_pack_start_map(2);
    yar_pack_push_string(pk, "name", 4);
    yar_pack_push_string(pk, "yar", 3);
    yar_pack_push_string(pk, "version", 7);
    yar_pack_push_long(pk, 1);
    yar_pack_to_string(pk, &payload);

    hex_dump("map{name:yar,version:1} msgpack", (unsigned char *)payload.data, payload.size);

    unpk = yar_unpack_init(payload.data, payload.size);
    data = yar_unpack_unpack(unpk);
    TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_MAP, "type is MAP");
    TEST_ASSERT(size == 2, "map size is 2");

    /* Iterate: map iterator yields key, value, key, value (size = entries * 2) */
    {
        yar_unpack_iterator *it = yar_unpack_iterator_init(data);
        const yar_data *elem;
        uint esize;
        const char *str;

        TEST_ASSERT(it != NULL, "map iterator created");

        /* Key 0: "name" */
        elem = yar_unpack_iterator_current(it);
        TEST_ASSERT(yar_unpack_data_type(elem, &esize) == YAR_DATA_STRING, "key[0] is STRING");
        yar_unpack_data_string(elem, &str);
        TEST_ASSERT(esize == 4 && memcmp(str, "name", 4) == 0, "key[0] is 'name'");

        /* Value 0: "yar" */
        yar_unpack_iterator_next(it);
        elem = yar_unpack_iterator_current(it);
        TEST_ASSERT(yar_unpack_data_type(elem, &esize) == YAR_DATA_STRING, "val[0] is STRING");
        yar_unpack_data_string(elem, &str);
        TEST_ASSERT(esize == 3 && memcmp(str, "yar", 3) == 0, "val[0] is 'yar'");

        /* Key 1: "version" */
        yar_unpack_iterator_next(it);
        elem = yar_unpack_iterator_current(it);
        TEST_ASSERT(yar_unpack_data_type(elem, &esize) == YAR_DATA_STRING, "key[1] is STRING");
        yar_unpack_data_string(elem, &str);
        TEST_ASSERT(esize == 7 && memcmp(str, "version", 7) == 0, "key[1] is 'version'");

        /* Value 1: 1 (int64 -> ULONG since >= 0) */
        yar_unpack_iterator_next(it);
        elem = yar_unpack_iterator_current(it);
        yar_data_type t = yar_unpack_data_type(elem, &esize);
        TEST_ASSERT(t == YAR_DATA_ULONG || t == YAR_DATA_LONG, "val[1] is numeric");
        ulong uval = 0;
        if (t == YAR_DATA_ULONG) {
            yar_unpack_data_ulong(elem, &uval);
        } else {
            long lv;
            yar_unpack_data_long(elem, &lv);
            uval = (ulong)lv;
        }
        TEST_ASSERT(uval == 1, "val[1] is 1");

        /* No more entries */
        TEST_ASSERT(yar_unpack_iterator_next(it) == 0, "no more map entries");

        yar_unpack_iterator_free(it);
    }

    yar_unpack_free(unpk);
    yar_pack_free(pk);
}

/* ---- Nested (map containing array) ---- */

static void test_pack_nested(void) {
    yar_packager *pk, *inner;
    yar_payload payload = {0};
    yar_unpackager *unpk;
    const yar_data *data;
    uint size = 0;

    printf("\n[test_pack_nested]\n");

    /* Build: {"items" => [1, 2, 3]} */
    inner = yar_pack_start_array(3);
    yar_pack_push_long(inner, 1);
    yar_pack_push_long(inner, 2);
    yar_pack_push_long(inner, 3);

    pk = yar_pack_start_map(1);
    yar_pack_push_string(pk, "items", 5);
    yar_pack_push_packager(pk, inner);
    yar_pack_to_string(pk, &payload);

    hex_dump("nested map{items:[1,2,3]} msgpack", (unsigned char *)payload.data, payload.size);

    unpk = yar_unpack_init(payload.data, payload.size);
    data = yar_unpack_unpack(unpk);
    TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_MAP, "outer type is MAP");
    TEST_ASSERT(size == 1, "outer map has 1 entry");

    {
        yar_unpack_iterator *it = yar_unpack_iterator_init(data);
        const yar_data *elem;
        uint esize;

        /* Key: "items" */
        elem = yar_unpack_iterator_current(it);
        TEST_ASSERT(yar_unpack_data_type(elem, &esize) == YAR_DATA_STRING, "key is STRING");

        /* Value: array [1, 2, 3] */
        yar_unpack_iterator_next(it);
        elem = yar_unpack_iterator_current(it);
        TEST_ASSERT(yar_unpack_data_type(elem, &esize) == YAR_DATA_ARRAY, "value is ARRAY");
        TEST_ASSERT(esize == 3, "inner array has 3 elements");

        /* Iterate inner array */
        {
            yar_unpack_iterator *inner_it = yar_unpack_iterator_init(elem);
            long expected[] = {1, 2, 3};
            int j;

            for (j = 0; j < 3; j++) {
                const yar_data *aelem = yar_unpack_iterator_current(inner_it);
                uint asize;
                yar_data_type at = yar_unpack_data_type(aelem, &asize);
                if (at == YAR_DATA_ULONG) {
                    ulong v;
                    yar_unpack_data_ulong(aelem, &v);
                    char msg[64];
                    snprintf(msg, sizeof(msg), "inner[%d] == %ld", j, expected[j]);
                    TEST_ASSERT((long)v == expected[j], msg);
                } else if (at == YAR_DATA_LONG) {
                    long v;
                    yar_unpack_data_long(aelem, &v);
                    char msg[64];
                    snprintf(msg, sizeof(msg), "inner[%d] == %ld", j, expected[j]);
                    TEST_ASSERT(v == expected[j], msg);
                }
                if (j < 2) yar_unpack_iterator_next(inner_it);
            }
            yar_unpack_iterator_free(inner_it);
        }

        yar_unpack_iterator_free(it);
    }

    yar_unpack_free(unpk);
    yar_pack_free(inner);
    yar_pack_free(pk);
}

/* ---- Push packager ---- */

static void test_pack_push_packager(void) {
    yar_packager *inner, *outer;
    yar_payload payload = {0};
    yar_unpackager *unpk;
    const yar_data *data;
    uint size = 0;

    printf("\n[test_pack_push_packager]\n");

    /*
     * Push a string packager into an array packager.
     * push_packager copies raw sbuffer bytes, so the inner packager's
     * serialized data becomes the next msgpack object in the outer buffer.
     */
    inner = yar_pack_start_string();
    yar_pack_push_string(inner, "pushed", 6);

    outer = yar_pack_start_array(1);
    yar_pack_push_packager(outer, inner);
    yar_pack_to_string(outer, &payload);

    hex_dump("push_packager result", (unsigned char *)payload.data, payload.size);

    unpk = yar_unpack_init(payload.data, payload.size);
    data = yar_unpack_unpack(unpk);
    TEST_ASSERT(yar_unpack_data_type(data, &size) == YAR_DATA_ARRAY, "outer type is ARRAY");
    TEST_ASSERT(size == 1, "outer array has 1 element");

    {
        yar_unpack_iterator *it = yar_unpack_iterator_init(data);
        const yar_data *elem = yar_unpack_iterator_current(it);
        const char *str = NULL;
        TEST_ASSERT(yar_unpack_data_type(elem, &size) == YAR_DATA_STRING, "elem is STRING");
        yar_unpack_data_string(elem, &str);
        TEST_ASSERT(size == 6 && memcmp(str, "pushed", 6) == 0, "elem is 'pushed'");
        yar_unpack_iterator_free(it);
    }

    yar_unpack_free(unpk);
    yar_pack_free(inner);
    yar_pack_free(outer);
}

/* ---- To string (raw bytes) ---- */

static void test_pack_to_string(void) {
    yar_packager *pk;
    yar_payload payload = {0};

    printf("\n[test_pack_to_string]\n");

    pk = yar_pack_start_long();
    yar_pack_push_long(pk, 12345);
    yar_pack_to_string(pk, &payload);

    TEST_ASSERT(payload.data != NULL, "payload data is non-null");
    TEST_ASSERT(payload.size > 0, "payload size is non-zero");
    /* Note: msgpack_pack_int64(12345) uses compact uint16 encoding (0xcd + 2 bytes = 3 bytes),
     * not full int64 (9 bytes), because 12345 fits in uint16 range. */
    TEST_ASSERT(payload.size >= 1 && payload.size <= 9,
        "long payload is 1-9 bytes (msgpack compact encoding)");

    hex_dump("to_string result", (unsigned char *)payload.data, payload.size);

    yar_pack_free(pk);
}

int main(void) {
    printf("=== yar_pack tests ===\n");

    test_pack_null();
    test_pack_bool_true();
    test_pack_bool_false();
    test_pack_long();
    test_pack_ulong();
    test_pack_double();
    test_pack_string();
    test_pack_array();
    test_pack_map();
    test_pack_nested();
    test_pack_push_packager();
    test_pack_to_string();

    printf("\n--- Results: %d/%d passed ---\n", tests_passed, tests_run);
    return (tests_passed == tests_run) ? 0 : 1;
}
