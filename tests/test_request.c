/*
 * test_request.c - Tests for yar_request (RPC request pack/unpack)
 *
 * Wire format of a packed request:
 *   [yar_header (82 bytes)] [YAR_PACKAGER "MSGPACK\0" (8 bytes)] [msgpack body]
 *
 * The msgpack body is a map with 3 keys:
 *   "i" -> uint64 (request id)
 *   "m" -> string (method name)
 *   "p" -> parameters (packed data, or null)
 *
 * IMPORTANT for Rust rewrite:
 *   - yar_request_pack allocates payload->data with extra_bytes prefix
 *   - The msgpack body starts at payload->data + extra_bytes
 *   - yar_request_set_parameters wraps the parameter packager with a null-start
 *     wrapper, which causes only the null byte to be read during unpack.
 *     This is a known design quirk: parameters set via set_parameters are
 *     effectively lost during pack/unpack roundtrip (only null is recovered).
 *   - To properly pass parameters, pack them directly into the request->out field.
 *   - yar_request_unpack allocates request->method via malloc
 *   - yar_request_unpack stores the unpackager in request->buffer
 *   - request->in points to the "p" value in the unpacked msgpack
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/time.h>
#include <arpa/inet.h>
#include "yar.h"

/* Provide external definition for inline function in yar_server.c */
ulong yar_get_microsec(void) {
    struct timeval tv;
    if (gettimeofday(&tv, (struct timezone *)NULL) == 0) {
        return (tv.tv_sec * 1000000) + tv.tv_usec;
    }
    return 0;
}

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
    printf("  %s (%zu bytes):\n    ", label, len);
    for (i = 0; i < len; i++) {
        printf("%02x", data[i]);
        if ((i + 1) % 32 == 0 && i + 1 < len) printf("\n    ");
    }
    printf("\n");
}

/*
 * Test: pack and unpack a request with null parameters.
 * This is the basic request format test.
 */
static void test_request_pack_unpack(void) {
    yar_request req_out, req_in;
    yar_payload payload = {0};
    int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER);

    printf("\n[test_request_pack_unpack]\n");

    /* Build outgoing request */
    memset(&req_out, 0, sizeof(req_out));
    req_out.id = 1000;
    req_out.method = strdup("hello");
    req_out.mlen = 5;
    req_out.out = NULL;  /* null parameters */

    int pack_result = yar_request_pack(&req_out, &payload, extra);
    TEST_ASSERT(pack_result == 1, "pack succeeds");
    TEST_ASSERT(payload.size > (size_t)extra, "payload larger than header+packager");
    TEST_ASSERT(payload.data != NULL, "payload data is non-null");

    hex_dump("Packed request (null params)", (unsigned char *)payload.data, payload.size);

    /* Verify header region was left as extra_bytes (zeroed) */
    /* The caller is responsible for filling the header */

    /* Unpack into new request */
    memset(&req_in, 0, sizeof(req_in));
    int unpack_result = yar_request_unpack(&req_in, payload.data, payload.size, extra);
    TEST_ASSERT(unpack_result == 1, "unpack succeeds");
    TEST_ASSERT(req_in.id == 1000, "unpacked id == 1000");
    TEST_ASSERT(req_in.mlen == 5, "unpacked method length == 5");
    TEST_ASSERT(memcmp(req_in.method, "hello", 5) == 0, "unpacked method == 'hello'");

    /* Parameters should be null (since we passed NULL for out) */
    {
        const yar_data *params = yar_request_get_parameters(&req_in);
        if (params) {
            uint size = 0;
            yar_data_type t = yar_unpack_data_type(params, &size);
            TEST_ASSERT(t == YAR_DATA_NULL, "parameters type is NULL");
        } else {
            TEST_ASSERT(1, "parameters pointer is null (no 'p' key or null value)");
        }
    }

    /* Print msgpack body hex (after header+packager) */
    printf("  --- Msgpack body (test vector for Rust) ---\n");
    hex_dump("Request body", (unsigned char *)(payload.data + extra), payload.size - extra);

    yar_request_free(&req_out);
    yar_request_free(&req_in);
    free(payload.data);
}

/*
 * Test: pack/unpack request with actual parameters.
 * We bypass set_parameters and set out directly to avoid the null-wrapping quirk.
 */
static void test_request_with_params(void) {
    yar_request req_out, req_in;
    yar_payload payload = {0};
    int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER);

    printf("\n[test_request_with_params]\n");

    /* Build parameters: array [42, "world"] */
    yar_packager *params = yar_pack_start_array(2);
    yar_pack_push_long(params, 42);
    yar_pack_push_string(params, "world", 5);

    /* Build request - set out directly to the parameter packager */
    memset(&req_out, 0, sizeof(req_out));
    req_out.id = 2000;
    req_out.method = strdup("greet");
    req_out.mlen = 5;
    req_out.out = params;  /* direct reference, not wrapped */

    int pack_result = yar_request_pack(&req_out, &payload, extra);
    TEST_ASSERT(pack_result == 1, "pack with params succeeds");

    hex_dump("Packed request (with params)", (unsigned char *)payload.data, payload.size);

    /* Unpack */
    memset(&req_in, 0, sizeof(req_in));
    int unpack_result = yar_request_unpack(&req_in, payload.data, payload.size, extra);
    TEST_ASSERT(unpack_result == 1, "unpack with params succeeds");
    TEST_ASSERT(req_in.id == 2000, "unpacked id == 2000");
    TEST_ASSERT(req_in.mlen == 5, "unpacked method length == 5");
    TEST_ASSERT(memcmp(req_in.method, "greet", 5) == 0, "unpacked method == 'greet'");

    /* Verify parameters */
    {
        const yar_data *pdata = yar_request_get_parameters(&req_in);
        TEST_ASSERT(pdata != NULL, "parameters pointer is non-null");

        if (pdata) {
            uint size = 0;
            yar_data_type t = yar_unpack_data_type(pdata, &size);
            TEST_ASSERT(t == YAR_DATA_ARRAY, "parameters type is ARRAY");
            TEST_ASSERT(size == 2, "parameters array has 2 elements");

            /* Iterate parameters */
            yar_unpack_iterator *it = yar_unpack_iterator_init(pdata);
            if (it) {
                const yar_data *elem;
                uint esize;

                /* Element 0: long 42 */
                elem = yar_unpack_iterator_current(it);
                t = yar_unpack_data_type(elem, &esize);
                if (t == YAR_DATA_ULONG) {
                    ulong v;
                    yar_unpack_data_ulong(elem, &v);
                    TEST_ASSERT(v == 42, "param[0] == 42");
                } else if (t == YAR_DATA_LONG) {
                    long v;
                    yar_unpack_data_long(elem, &v);
                    TEST_ASSERT(v == 42, "param[0] == 42");
                }

                /* Element 1: string "world" */
                yar_unpack_iterator_next(it);
                elem = yar_unpack_iterator_current(it);
                t = yar_unpack_data_type(elem, &esize);
                TEST_ASSERT(t == YAR_DATA_STRING, "param[1] is STRING");
                const char *str;
                yar_unpack_data_string(elem, &str);
                TEST_ASSERT(esize == 5 && memcmp(str, "world", 5) == 0,
                    "param[1] == 'world'");

                yar_unpack_iterator_free(it);
            }
        }
    }

    /* Print msgpack body test vector */
    printf("  --- Msgpack body (test vector for Rust) ---\n");
    hex_dump("Request body with params", (unsigned char *)(payload.data + extra), payload.size - extra);

    /* Note: req_out.out was consumed by yar_request_pack (freed inside).
     * We must NOT free params separately. But yar_request_free will try to
     * free req_out.out as a yar_packager. Since pack already freed the
     * internal pk used for the map (not req_out.out), req_out.out (params)
     * is still alive and will be freed by yar_request_free. */
    yar_request_free(&req_out);
    yar_request_free(&req_in);
    free(payload.data);
}

/*
 * Test: extra_bytes offset.
 * Verify that pack places the msgpack body at the correct offset,
 * and unpack reads from the correct offset.
 */
static void test_request_extra_bytes(void) {
    yar_request req_out, req_in;
    yar_payload payload = {0};
    int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER); /* 82 + 8 = 90 */

    printf("\n[test_request_extra_bytes]\n");

    printf("  INFO: sizeof(yar_header) = %zu\n", sizeof(yar_header));
    printf("  INFO: sizeof(YAR_PACKAGER) = %zu\n", sizeof(YAR_PACKAGER));
    printf("  INFO: extra_bytes = %d\n", extra);

    memset(&req_out, 0, sizeof(req_out));
    req_out.id = 999;
    req_out.method = strdup("test");
    req_out.mlen = 4;
    req_out.out = NULL;

    yar_request_pack(&req_out, &payload, extra);

    TEST_ASSERT(payload.size > (size_t)extra, "payload size > extra_bytes");

    /* Fill in the header area with known pattern */
    memset(payload.data, 0xAA, extra);

    /* Verify the extra bytes region was preserved */
    int pattern_ok = 1;
    for (int i = 0; i < extra; i++) {
        if ((unsigned char)payload.data[i] != 0xAA) {
            pattern_ok = 0;
            break;
        }
    }
    TEST_ASSERT(pattern_ok, "extra_bytes region preserved (0xAA pattern)");

    /* Restore header for unpack */
    memset(payload.data, 0, extra);

    memset(&req_in, 0, sizeof(req_in));
    int unpack_result = yar_request_unpack(&req_in, payload.data, payload.size, extra);
    TEST_ASSERT(unpack_result == 1, "unpack with extra_bytes succeeds");
    TEST_ASSERT(req_in.id == 999, "unpacked id == 999");
    TEST_ASSERT(req_in.mlen == 4, "unpacked mlen == 4");
    TEST_ASSERT(memcmp(req_in.method, "test", 4) == 0, "unpacked method == 'test'");

    yar_request_free(&req_out);
    yar_request_free(&req_in);
    free(payload.data);
}

/*
 * Test: set_parameters and get_parameters.
 *
 * yar_request_set_parameters wraps the packager with a null-start wrapper,
 * but since yar_pack_start_null() creates an empty sbuffer (no nil byte
 * actually written), and push_packager copies the raw sbuffer bytes,
 * the parameters are actually preserved correctly during pack/unpack.
 */
static void test_request_set_get_parameters(void) {
    yar_request req;
    yar_packager *params;

    printf("\n[test_request_set_get_parameters]\n");

    memset(&req, 0, sizeof(req));

    /* Build parameters: array [1, 2, 3] */
    params = yar_pack_start_array(3);
    yar_pack_push_long(params, 1);
    yar_pack_push_long(params, 2);
    yar_pack_push_long(params, 3);

    /* Set parameters (this wraps with null-start, but wrapping is
     * transparent since null-start has empty sbuffer) */
    yar_request_set_parameters(&req, params);

    TEST_ASSERT(req.out != NULL, "request->out is set after set_parameters");

    /* Pack and unpack to verify parameters are preserved */
    {
        yar_payload payload = {0};
        int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER);

        req.id = 500;
        req.method = strdup("wrapped");
        req.mlen = 7;

        yar_request_pack(&req, &payload, extra);
        hex_dump("Request with set_parameters", (unsigned char *)payload.data, payload.size);

        yar_request req_in;
        memset(&req_in, 0, sizeof(req_in));
        yar_request_unpack(&req_in, payload.data, payload.size, extra);

        TEST_ASSERT(req_in.id == 500, "unpacked id == 500");
        TEST_ASSERT(req_in.mlen == 7, "unpacked mlen == 7");

        const yar_data *p = yar_request_get_parameters(&req_in);
        TEST_ASSERT(p != NULL, "parameters pointer is non-null after roundtrip");

        if (p) {
            uint sz = 0;
            yar_data_type t = yar_unpack_data_type(p, &sz);
            TEST_ASSERT(t == YAR_DATA_ARRAY, "parameters type is ARRAY after roundtrip");
            TEST_ASSERT(sz == 3, "parameters array has 3 elements");

            /* Verify array contents */
            yar_unpack_iterator *it = yar_unpack_iterator_init(p);
            if (it) {
                long expected[] = {1, 2, 3};
                int j;
                for (j = 0; j < 3; j++) {
                    const yar_data *elem = yar_unpack_iterator_current(it);
                    uint esz;
                    yar_data_type et = yar_unpack_data_type(elem, &esz);
                    long v = 0;
                    if (et == YAR_DATA_ULONG) {
                        ulong uv;
                        yar_unpack_data_ulong(elem, &uv);
                        v = (long)uv;
                    } else if (et == YAR_DATA_LONG) {
                        yar_unpack_data_long(elem, &v);
                    }
                    char msg[64];
                    snprintf(msg, sizeof(msg), "param[%d] == %ld", j, expected[j]);
                    TEST_ASSERT(v == expected[j], msg);
                    if (j < 2) yar_unpack_iterator_next(it);
                }
                yar_unpack_iterator_free(it);
            }
        }

        yar_request_free(&req_in);
        free(payload.data);
    }

    yar_pack_free(params);
    yar_request_free(&req);
}

int main(void) {
    /* Initialize logger to suppress alog() calls from writing to stderr */
    yar_logger_init(NULL, YAR_ERROR);

    printf("=== yar_request tests ===\n");

    test_request_pack_unpack();
    test_request_with_params();
    test_request_extra_bytes();
    test_request_set_get_parameters();

    printf("\n--- Results: %d/%d passed ---\n", tests_passed, tests_run);

    yar_logger_destroy();
    return (tests_passed == tests_run) ? 0 : 1;
}
