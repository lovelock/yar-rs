/*
 * cross_client.c - Cross-language demo client for yar RPC
 *
 * Connects to a yar server and calls three methods:
 *   echo  (no params)       - expects "echo:ok"
 *   add   (params: 10, 20)  - expects 30
 *   greet (param: "Yar")    - expects "Hello, Yar!"
 *
 * Usage: ./cross_client -h host:port [-n num_calls]
 *   -h host:port  Server address (required)
 *   -n num_calls  Repeat the 3 calls N times (default: 1)
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/time.h>
#include <getopt.h>
#include "yar.h"

/* Provide extern definition required by yar_server.c (linked in) */
ulong yar_get_microsec(void) {
    struct timeval tv;
    if (gettimeofday(&tv, (struct timezone *)NULL) == 0) {
        return (tv.tv_sec * 1000000) + tv.tv_usec;
    }
    return 0;
}

/*
 * Print a response value in a human-readable format.
 */
static void print_response(const char *label, yar_response *response) {
    if (!yar_response_get_status(response)) {
        uint size = 0;
        const yar_data *data = yar_response_get_response(response);
        if (data) {
            yar_data_type t = yar_unpack_data_type(data, &size);
            switch (t) {
                case YAR_DATA_STRING: {
                    const char *str;
                    yar_unpack_data_string(data, &str);
                    printf("%s => %.*s\n", label, size, str);
                    break;
                }
                case YAR_DATA_LONG: {
                    long num = 0;
                    yar_unpack_data_long(data, &num);
                    printf("%s => %ld\n", label, num);
                    break;
                }
                case YAR_DATA_ULONG: {
                    ulong num = 0;
                    yar_unpack_data_ulong(data, &num);
                    printf("%s => %lu\n", label, num);
                    break;
                }
                default:
                    printf("%s => (type %d, size %u)\n", label, t, size);
                    break;
            }
        } else {
            printf("%s => (null)\n", label);
        }
    } else {
        const char *msg = NULL;
        uint len = 0;
        yar_response_get_error(response, &msg, &len);
        printf("%s => ERROR: %.*s\n", label, len, msg);
    }
}

/*
 * Call echo method (no parameters).
 */
static int call_echo(yar_client *client) {
    yar_response *response = client->call(client, "echo", 0, NULL);
    if (!response) {
        fprintf(stderr, "[CALL echo] => no response\n");
        return -1;
    }
    print_response("[CALL echo]", response);
    yar_response_free(response);
    free(response);
    return 0;
}

/*
 * Call add method with two integer parameters: 10 + 20.
 */
static int call_add(yar_client *client) {
    yar_packager *args[2];
    yar_response *response;

    args[0] = yar_pack_start_long();
    yar_pack_push_long(args[0], 10);

    args[1] = yar_pack_start_long();
    yar_pack_push_long(args[1], 20);

    response = client->call(client, "add", 2, args);

    yar_pack_free(args[0]);
    yar_pack_free(args[1]);

    if (!response) {
        fprintf(stderr, "[CALL add 10+20] => no response\n");
        return -1;
    }
    print_response("[CALL add 10+20]", response);
    yar_response_free(response);
    free(response);
    return 0;
}

/*
 * Call greet method with a string parameter: "Yar".
 */
static int call_greet(yar_client *client) {
    yar_packager *args[1];
    yar_response *response;

    args[0] = yar_pack_start_string();
    yar_pack_push_string(args[0], "Yar", 3);

    response = client->call(client, "greet", 1, args);

    yar_pack_free(args[0]);

    if (!response) {
        fprintf(stderr, "[CALL greet \"Yar\"] => no response\n");
        return -1;
    }
    print_response("[CALL greet \"Yar\"]", response);
    yar_response_free(response);
    free(response);
    return 0;
}

static void print_usage(const char *prog) {
    fprintf(stderr, "Usage: %s -h <host:port> [-n <num_calls>]\n", prog);
    fprintf(stderr, "  -h host:port  Server address (required)\n");
    fprintf(stderr, "  -n num_calls  Repeat the 3 calls N times (default: 1)\n");
}

int main(int argc, char **argv) {
    int opt;
    char *hostname = NULL;
    int num_calls = 1;

    while ((opt = getopt(argc, argv, "h:n:")) != -1) {
        switch (opt) {
            case 'h':
                hostname = optarg;
                break;
            case 'n':
                num_calls = atoi(optarg);
                if (num_calls < 1) num_calls = 1;
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

    yar_client *client = yar_client_init(hostname);
    if (!client) {
        fprintf(stderr, "Failed to connect to %s\n", hostname);
        return 1;
    }

    int persistent = 1;
    int timeout = 5;
    yar_client_set_opt(client, YAR_PERSISTENT_LINK, &persistent);
    yar_client_set_opt(client, YAR_CONNECT_TIMEOUT, &timeout);

    int i;
    int errors = 0;
    for (i = 0; i < num_calls; i++) {
        if (num_calls > 1) {
            printf("--- round %d/%d ---\n", i + 1, num_calls);
        }
        errors += call_echo(client)  < 0 ? 1 : 0;
        errors += call_add(client)   < 0 ? 1 : 0;
        errors += call_greet(client) < 0 ? 1 : 0;
    }

    yar_client_destroy(client);

    if (errors > 0) {
        fprintf(stderr, "\n%d call(s) failed\n", errors);
        return 1;
    }
    return 0;
}
