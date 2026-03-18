# Comment Ratio -- C

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .c, .h
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```c
int parse_header(const uint8_t *buf, size_t len, struct header *out)
{
    if (len < 4)
        return -1;

    uint16_t magic = (buf[0] << 8) | buf[1];
    if (magic != 0xCAFE && magic != 0xBEEF)
        return -2;

    uint8_t version = buf[2];
    uint8_t flags = buf[3];
    size_t offset = 4;

    if (flags & FLAG_EXTENDED) {
        if (len < offset + 8)
            return -1;
        out->extended_size = 0;
        for (int i = 0; i < 8; i++) {
            out->extended_size |= ((uint64_t)buf[offset + i]) << (56 - i * 8);
        }
        offset += 8;
    }

    if (version >= 3) {
        if (len < offset + 2)
            return -1;
        out->num_entries = (buf[offset] << 8) | buf[offset + 1];
        offset += 2;
        if (out->num_entries > MAX_ENTRIES)
            return -3;
    } else {
        out->num_entries = buf[offset];
        offset += 1;
    }

    if (flags & FLAG_CHECKSUM) {
        if (len < offset + 4)
            return -1;
        uint32_t expected = 0;
        for (int i = 0; i < 4; i++)
            expected |= ((uint32_t)buf[offset + i]) << (24 - i * 8);
        uint32_t actual = crc32(buf, offset);
        if (expected != actual)
            return -4;
    }

    out->version = version;
    out->flags = flags;
    return 0;
}
```

### Good Code (Fix)
```c
/*
 * Parse a binary header from buf into out.
 * Returns 0 on success, negative error codes on failure:
 *   -1 = truncated input, -2 = bad magic, -3 = too many entries, -4 = CRC mismatch
 */
int parse_header(const uint8_t *buf, size_t len, struct header *out)
{
    if (len < 4)
        return -1;

    /* Two valid magic values: 0xCAFE (v1 format) and 0xBEEF (v2+ format) */
    uint16_t magic = (buf[0] << 8) | buf[1];
    if (magic != 0xCAFE && magic != 0xBEEF)
        return -2;

    uint8_t version = buf[2];
    uint8_t flags = buf[3];
    size_t offset = 4;

    if (flags & FLAG_EXTENDED) {
        if (len < offset + 8)
            return -1;
        /* Extended size is big-endian uint64 -- used for payloads > 4GB */
        out->extended_size = 0;
        for (int i = 0; i < 8; i++) {
            out->extended_size |= ((uint64_t)buf[offset + i]) << (56 - i * 8);
        }
        offset += 8;
    }

    /* v3+ uses 16-bit entry count; older versions use single byte */
    if (version >= 3) {
        if (len < offset + 2)
            return -1;
        out->num_entries = (buf[offset] << 8) | buf[offset + 1];
        offset += 2;
        if (out->num_entries > MAX_ENTRIES)
            return -3;
    } else {
        out->num_entries = buf[offset];
        offset += 1;
    }

    if (flags & FLAG_CHECKSUM) {
        if (len < offset + 4)
            return -1;
        /* CRC covers everything before the checksum field itself */
        uint32_t expected = 0;
        for (int i = 0; i < 4; i++)
            expected |= ((uint32_t)buf[offset + i]) << (24 - i * 8);
        uint32_t actual = crc32(buf, offset);
        if (expected != actual)
            return -4;
    }

    out->version = version;
    out->flags = flags;
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition` for function bodies; `comment` for `//` and `/* */`
- **Detection approach**: Count comment lines and code lines within a function body. Calculate ratio. Flag functions with CC > 5 and comment ratio below threshold.
- **S-expression query sketch**:
  ```scheme
  ;; Capture function body and any comments within it
  (function_definition
    body: (compound_statement) @function.body)

  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `undocumented_complex_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Trivial Over-Commenting

### Description
Functions with comments that merely restate the code rather than explaining intent, adding noise without value.

### Bad Code (Anti-pattern)
```c
int init_buffer(struct buffer *buf, size_t size)
{
    /* Allocate memory */
    buf->data = malloc(size);

    /* Check if allocation failed */
    if (buf->data == NULL) {
        /* Return error */
        return -1;
    }

    /* Set the size */
    buf->size = size;

    /* Set the length to zero */
    buf->len = 0;

    /* Initialize the mutex */
    pthread_mutex_init(&buf->lock, NULL);

    /* Return success */
    return 0;
}
```

### Good Code (Fix)
```c
int init_buffer(struct buffer *buf, size_t size)
{
    buf->data = malloc(size);
    if (buf->data == NULL)
        return -1;

    buf->size = size;
    buf->len = 0;

    /* Mutex needed because consumer threads may read while producer appends */
    pthread_mutex_init(&buf->lock, NULL);

    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment` adjacent to `expression_statement`, `declaration`, `return_statement`, `if_statement`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (compound_statement
    (comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
