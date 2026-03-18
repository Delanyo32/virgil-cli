# Function Length -- C

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Threshold**: 50 lines

---

## Pattern 1: Oversized Function Body

### Description
A function exceeding the 50-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```c
int process_packet(int sockfd, struct config *cfg, struct stats *st)
{
    /* Read packet header */
    uint8_t header[16];
    ssize_t n = recv(sockfd, header, sizeof(header), 0);
    if (n < 0) {
        perror("recv header");
        return -1;
    }
    if (n < (ssize_t)sizeof(header)) {
        fprintf(stderr, "Short read on header: %zd bytes\n", n);
        return -1;
    }

    /* Parse header fields */
    uint32_t magic = ntohl(*(uint32_t *)&header[0]);
    if (magic != PACKET_MAGIC) {
        fprintf(stderr, "Invalid magic: 0x%08x\n", magic);
        st->invalid_packets++;
        return -1;
    }
    uint16_t version = ntohs(*(uint16_t *)&header[4]);
    if (version < MIN_VERSION || version > MAX_VERSION) {
        fprintf(stderr, "Unsupported version: %u\n", version);
        st->invalid_packets++;
        return -1;
    }
    uint16_t type = ntohs(*(uint16_t *)&header[6]);
    uint32_t payload_len = ntohl(*(uint32_t *)&header[8]);
    uint32_t checksum = ntohl(*(uint32_t *)&header[12]);

    /* Validate payload length */
    if (payload_len > cfg->max_payload) {
        fprintf(stderr, "Payload too large: %u > %u\n", payload_len, cfg->max_payload);
        st->oversized_packets++;
        return -1;
    }

    /* Read payload */
    uint8_t *payload = malloc(payload_len);
    if (!payload) {
        perror("malloc payload");
        return -1;
    }
    size_t total_read = 0;
    while (total_read < payload_len) {
        n = recv(sockfd, payload + total_read, payload_len - total_read, 0);
        if (n <= 0) {
            perror("recv payload");
            free(payload);
            return -1;
        }
        total_read += n;
    }

    /* Verify checksum */
    uint32_t computed = crc32(0, payload, payload_len);
    if (computed != checksum) {
        fprintf(stderr, "Checksum mismatch: expected 0x%08x, got 0x%08x\n", checksum, computed);
        st->checksum_errors++;
        free(payload);
        return -1;
    }

    /* Process by type */
    int result = 0;
    switch (type) {
        case TYPE_DATA:
            result = handle_data(payload, payload_len, cfg);
            st->data_packets++;
            break;
        case TYPE_CONTROL:
            result = handle_control(payload, payload_len, cfg);
            st->control_packets++;
            break;
        case TYPE_KEEPALIVE:
            st->keepalive_packets++;
            break;
        default:
            fprintf(stderr, "Unknown packet type: %u\n", type);
            st->unknown_packets++;
            result = -1;
            break;
    }

    /* Update statistics */
    st->total_packets++;
    st->total_bytes += sizeof(header) + payload_len;
    if (result == 0) {
        st->successful_packets++;
    } else {
        st->failed_packets++;
    }

    /* Log if configured */
    if (cfg->log_packets) {
        FILE *log = fopen(cfg->log_path, "a");
        if (log) {
            fprintf(log, "%ld type=%u len=%u result=%d\n", time(NULL), type, payload_len, result);
            fclose(log);
        }
    }

    free(payload);
    return result;
}
```

### Good Code (Fix)
```c
static int read_packet_header(int sockfd, struct packet_header *hdr)
{
    uint8_t raw[16];
    ssize_t n = recv(sockfd, raw, sizeof(raw), 0);
    if (n < (ssize_t)sizeof(raw)) {
        perror("recv header");
        return -1;
    }
    hdr->magic = ntohl(*(uint32_t *)&raw[0]);
    hdr->version = ntohs(*(uint16_t *)&raw[4]);
    hdr->type = ntohs(*(uint16_t *)&raw[6]);
    hdr->payload_len = ntohl(*(uint32_t *)&raw[8]);
    hdr->checksum = ntohl(*(uint32_t *)&raw[12]);
    return 0;
}

static int validate_header(const struct packet_header *hdr, const struct config *cfg, struct stats *st)
{
    if (hdr->magic != PACKET_MAGIC) {
        st->invalid_packets++;
        return -1;
    }
    if (hdr->version < MIN_VERSION || hdr->version > MAX_VERSION) {
        st->invalid_packets++;
        return -1;
    }
    if (hdr->payload_len > cfg->max_payload) {
        st->oversized_packets++;
        return -1;
    }
    return 0;
}

static uint8_t *read_payload(int sockfd, uint32_t len)
{
    uint8_t *buf = malloc(len);
    if (!buf) return NULL;
    size_t total = 0;
    while (total < len) {
        ssize_t n = recv(sockfd, buf + total, len - total, 0);
        if (n <= 0) { free(buf); return NULL; }
        total += n;
    }
    return buf;
}

static int dispatch_packet(uint16_t type, uint8_t *payload, uint32_t len,
                           struct config *cfg, struct stats *st)
{
    switch (type) {
        case TYPE_DATA:      st->data_packets++;      return handle_data(payload, len, cfg);
        case TYPE_CONTROL:   st->control_packets++;   return handle_control(payload, len, cfg);
        case TYPE_KEEPALIVE: st->keepalive_packets++; return 0;
        default:             st->unknown_packets++;   return -1;
    }
}

int process_packet(int sockfd, struct config *cfg, struct stats *st)
{
    struct packet_header hdr;
    if (read_packet_header(sockfd, &hdr) < 0) return -1;
    if (validate_header(&hdr, cfg, st) < 0) return -1;

    uint8_t *payload = read_payload(sockfd, hdr.payload_len);
    if (!payload) return -1;

    if (crc32(0, payload, hdr.payload_len) != hdr.checksum) {
        st->checksum_errors++;
        free(payload);
        return -1;
    }

    int result = dispatch_packet(hdr.type, payload, hdr.payload_len, cfg, st);
    update_stats(st, &hdr, result);
    log_packet(cfg, &hdr, result);
    free(payload);
    return result;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 50.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @func.name)
    body: (compound_statement) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `oversized_function`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Handler/Entry Point

### Description
A single `main()` function or event-loop callback that contains all program logic inline -- initialization, processing, cleanup, error handling -- instead of delegating to focused functions.

### Bad Code (Anti-pattern)
```c
int main(int argc, char *argv[])
{
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <host> <port>\n", argv[0]);
        return EXIT_FAILURE;
    }
    const char *host = argv[1];
    int port = atoi(argv[2]);
    if (port <= 0 || port > 65535) {
        fprintf(stderr, "Invalid port: %s\n", argv[2]);
        return EXIT_FAILURE;
    }
    int sockfd = socket(AF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) {
        perror("socket");
        return EXIT_FAILURE;
    }
    int opt = 1;
    setsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    if (inet_pton(AF_INET, host, &addr.sin_addr) <= 0) {
        fprintf(stderr, "Invalid address: %s\n", host);
        close(sockfd);
        return EXIT_FAILURE;
    }
    if (bind(sockfd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        perror("bind");
        close(sockfd);
        return EXIT_FAILURE;
    }
    if (listen(sockfd, SOMAXCONN) < 0) {
        perror("listen");
        close(sockfd);
        return EXIT_FAILURE;
    }
    printf("Listening on %s:%d\n", host, port);
    struct config cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.max_payload = 1024 * 1024;
    cfg.log_packets = 1;
    snprintf(cfg.log_path, sizeof(cfg.log_path), "/var/log/server.log");
    struct stats st;
    memset(&st, 0, sizeof(st));
    while (1) {
        struct sockaddr_in client_addr;
        socklen_t client_len = sizeof(client_addr);
        int client_fd = accept(sockfd, (struct sockaddr *)&client_addr, &client_len);
        if (client_fd < 0) {
            perror("accept");
            continue;
        }
        char client_ip[INET_ADDRSTRLEN];
        inet_ntop(AF_INET, &client_addr.sin_addr, client_ip, sizeof(client_ip));
        printf("Connection from %s\n", client_ip);
        while (process_packet(client_fd, &cfg, &st) == 0)
            ;
        close(client_fd);
        printf("Client %s disconnected. Stats: %lu packets\n", client_ip, st.total_packets);
    }
    close(sockfd);
    return EXIT_SUCCESS;
}
```

### Good Code (Fix)
```c
static int parse_args(int argc, char *argv[], const char **host, int *port)
{
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <host> <port>\n", argv[0]);
        return -1;
    }
    *host = argv[1];
    *port = atoi(argv[2]);
    if (*port <= 0 || *port > 65535) return -1;
    return 0;
}

static int create_listener(const char *host, int port)
{
    int sockfd = socket(AF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) return -1;
    int opt = 1;
    setsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
    struct sockaddr_in addr = {0};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    if (inet_pton(AF_INET, host, &addr.sin_addr) <= 0) { close(sockfd); return -1; }
    if (bind(sockfd, (struct sockaddr *)&addr, sizeof(addr)) < 0) { close(sockfd); return -1; }
    if (listen(sockfd, SOMAXCONN) < 0) { close(sockfd); return -1; }
    return sockfd;
}

static void handle_client(int client_fd, struct config *cfg, struct stats *st)
{
    while (process_packet(client_fd, cfg, st) == 0)
        ;
    close(client_fd);
}

int main(int argc, char *argv[])
{
    const char *host;
    int port;
    if (parse_args(argc, argv, &host, &port) < 0) return EXIT_FAILURE;

    int sockfd = create_listener(host, port);
    if (sockfd < 0) return EXIT_FAILURE;

    struct config cfg = default_config();
    struct stats st = {0};
    accept_loop(sockfd, &cfg, &st);

    close(sockfd);
    return EXIT_SUCCESS;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 50. The `main()` function and callback functions are detected the same way as regular functions; the pattern name differentiates them in reporting.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @func.name)
    body: (compound_statement) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
