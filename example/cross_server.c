/*
 * cross_server.c - Cross-language demo server for yar RPC
 *
 * Registers three handlers:
 *   echo  - returns "echo:ok"
 *   add   - takes two integer params, returns their sum
 *   greet - takes a string param, returns "Hello, {name}!"
 *
 * Usage: ./cross_server -S host:port [-X]
 *   -S host:port  Bind address (required)
 *   -X            Standalone mode (default: on)
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/time.h>
#include <getopt.h>
#include "yar.h"

/* Provide extern definition required by yar_server.c */
ulong yar_get_microsec(void) {
    struct timeval tv;
    if (gettimeofday(&tv, (struct timezone *)NULL) == 0) {
        return (tv.tv_sec * 1000000) + tv.tv_usec;
    }
    return 0;
}

/*
 * echo handler: returns the string "echo:ok"
 */
static void echo_handler(yar_request *request, yar_response *response, void *data) {
    (void)request;
    (void)data;

    yar_packager *retval = yar_pack_start_string();
    yar_pack_push_string(retval, "echo:ok", 7);

    if (response->out) {
        yar_pack_free((yar_packager *)response->out);
    }
    response->out = retval;
}

/*
 * add handler: takes two integer params, returns their sum as a long
 */
static void add_handler(yar_request *request, yar_response *response, void *data) {
    (void)data;

    long a = 0, b = 0;
    const yar_data *parameters = yar_request_get_parameters(request);

    if (parameters) {
        uint size = 0;
        yar_data_type t = yar_unpack_data_type(parameters, &size);
        if (t == YAR_DATA_ARRAY && size >= 2) {
            yar_unpack_iterator *it = yar_unpack_iterator_init(parameters);
            if (it) {
                const yar_data *elem;
                /* Iterator: current() first, then next() to advance */
                elem = yar_unpack_iterator_current(it);
                if (elem) {
                    uint esize = 0;
                    yar_data_type et = yar_unpack_data_type(elem, &esize);
                    if (et == YAR_DATA_ULONG) {
                        ulong val;
                        yar_unpack_data_ulong(elem, &val);
                        a = (long)val;
                    } else {
                        yar_unpack_data_long(elem, &a);
                    }
                }
                if (yar_unpack_iterator_next(it)) {
                    elem = yar_unpack_iterator_current(it);
                    if (elem) {
                        uint esize = 0;
                        yar_data_type et = yar_unpack_data_type(elem, &esize);
                        if (et == YAR_DATA_ULONG) {
                            ulong val;
                            yar_unpack_data_ulong(elem, &val);
                            b = (long)val;
                        } else {
                            yar_unpack_data_long(elem, &b);
                        }
                    }
                }
                yar_unpack_iterator_free(it);
            }
        }
    }

    yar_packager *retval = yar_pack_start_long();
    yar_pack_push_long(retval, a + b);

    if (response->out) {
        yar_pack_free((yar_packager *)response->out);
    }
    response->out = retval;
}

/*
 * greet handler: takes a string param, returns "Hello, {name}!"
 */
static void greet_handler(yar_request *request, yar_response *response, void *data) {
    (void)data;

    char greeting[512];
    const char *name = "World";
    const yar_data *parameters = yar_request_get_parameters(request);

    if (parameters) {
        uint size = 0;
        yar_data_type t = yar_unpack_data_type(parameters, &size);

        /* Try to extract string from array or direct string */
        if (t == YAR_DATA_ARRAY && size >= 1) {
            yar_unpack_iterator *it = yar_unpack_iterator_init(parameters);
            if (it) {
                /* Iterator: current() first, then next() to advance */
                const yar_data *elem = yar_unpack_iterator_current(it);
                if (elem) {
                    uint slen = 0;
                    yar_data_type et = yar_unpack_data_type(elem, &slen);
                    if (et == YAR_DATA_STRING) {
                        const char *str;
                        yar_unpack_data_string(elem, &str);
                        snprintf(greeting, sizeof(greeting), "Hello, %.*s!", slen, str);
                        name = NULL;
                    }
                }
                yar_unpack_iterator_free(it);
            }
        } else if (t == YAR_DATA_STRING) {
            const char *str;
            yar_unpack_data_string(parameters, &str);
            snprintf(greeting, sizeof(greeting), "Hello, %.*s!", size, str);
            name = NULL;
        }
    }

    yar_packager *retval = yar_pack_start_string();
    if (name) {
        snprintf(greeting, sizeof(greeting), "Hello, %s!", name);
    }
    yar_pack_push_string(retval, greeting, strlen(greeting));

    if (response->out) {
        yar_pack_free((yar_packager *)response->out);
    }
    response->out = retval;
}

/* Null-terminated handler table */
static yar_server_handler cross_handlers[] = {
    {"echo",   4, echo_handler},
    {"add",    3, add_handler},
    {"greet",  5, greet_handler},
    {NULL,     0, NULL}
};

static void print_usage(const char *prog) {
    fprintf(stderr, "Usage: %s -S <host:port> [-X]\n", prog);
    fprintf(stderr, "  -S host:port  Bind address (required)\n");
    fprintf(stderr, "  -X            Standalone mode (default: on)\n");
}

int main(int argc, char **argv) {
    int opt;
    char *hostname = NULL;
    int standalone = 1;

    while ((opt = getopt(argc, argv, "S:X")) != -1) {
        switch (opt) {
            case 'S':
                hostname = optarg;
                break;
            case 'X':
                standalone = 1;
                break;
            default:
                print_usage(argv[0]);
                return 1;
        }
    }

    if (!hostname) {
        print_usage(argv[0]);
        return 1;
    }

    if (yar_server_init(hostname)) {
        yar_server_set_opt(YAR_STAND_ALONE, &standalone);
        yar_server_register_handler(cross_handlers);

        printf("cross_server: listening on %s (standalone=%d)\n", hostname, standalone);
        yar_server_run();
    } else {
        fprintf(stderr, "cross_server: failed to init on %s\n", hostname);
        return 1;
    }

    return 0;
}
