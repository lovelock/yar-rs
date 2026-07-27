CC = gcc
CFLAGS = -I../legacy -I/opt/homebrew/include -g -Wall -Wno-unused-result
LDFLAGS = -L/opt/homebrew/lib -lmsgpackc -levent -lm

# Link against the yar source files from legacy/
YAR_SRCS = ../legacy/yar_protocol.c ../legacy/yar_pack.c ../legacy/yar_request.c ../legacy/yar_response.c \
           ../legacy/yar_log.c ../legacy/yar_client.c ../legacy/yar_server.c

TARGETS = cross_server cross_client

all: $(TARGETS)

cross_server: cross_server.c $(YAR_SRCS)
	$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS)

cross_client: cross_client.c $(YAR_SRCS)
	$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS)

clean:
	rm -f $(TARGETS)

.PHONY: all clean
