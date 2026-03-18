# Magic Values -- C++

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```cpp
Response processRequest(const std::vector<uint8_t>& data) {
    if (data.size() > 1024) {
        return Response(413);
    }
    for (int i = 0; i < 3; ++i) {
        std::this_thread::sleep_for(std::chrono::seconds(86400));
    }
    if (response.statusCode() == 200) {
        cache.insert(key, data, 3600);
    } else if (response.statusCode() == 404) {
        return Response::notFound();
    }
    return Response::ok();
}
```

### Good Code (Fix)
```cpp
constexpr int MAX_PAYLOAD_SIZE = 1024;
constexpr int MAX_RETRIES = 3;
constexpr int SECONDS_PER_DAY = 86400;
constexpr int HTTP_OK = 200;
constexpr int HTTP_NOT_FOUND = 404;
constexpr int CACHE_TTL_SECONDS = 3600;

Response processRequest(const std::vector<uint8_t>& data) {
    if (data.size() > MAX_PAYLOAD_SIZE) {
        return Response(HTTP_PAYLOAD_TOO_LARGE);
    }
    for (int i = 0; i < MAX_RETRIES; ++i) {
        std::this_thread::sleep_for(std::chrono::seconds(SECONDS_PER_DAY));
    }
    if (response.statusCode() == HTTP_OK) {
        cache.insert(key, data, CACHE_TTL_SECONDS);
    } else if (response.statusCode() == HTTP_NOT_FOUND) {
        return Response::notFound();
    }
    return Response::ok();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `number_literal` (excludes 0, 1, -1)
- **Detection approach**: Find `number_literal` nodes in expressions. Exclude literals inside `preproc_def`, `preproc_function_def`, `enumerator`, or `template_argument_list` ancestor nodes, and `declaration` ancestors with a `const` type qualifier or `constexpr` storage class specifier. Also exclude `subscript_argument_list` positions (array indexing). Flag literals that are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
(number_literal) @number
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_numeric_literal`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Magic Strings

### Description
String literals used for comparisons, dictionary keys, or status values without named constants -- prone to typos and hard to refactor.

### Bad Code (Anti-pattern)
```cpp
void handleCommand(const std::string& cmd) {
    if (cmd == "start") {
        startService();
    } else if (cmd == "stop") {
        stopService();
    } else if (cmd == "restart") {
        restartService();
    }
    auto dbUrl = config["database_url"];
    if (status == "active") {
        activate();
    }
}
```

### Good Code (Fix)
```cpp
constexpr const char* CMD_START = "start";
constexpr const char* CMD_STOP = "stop";
constexpr const char* CMD_RESTART = "restart";
constexpr const char* STATUS_ACTIVE = "active";
constexpr const char* CONFIG_DATABASE_URL = "database_url";

void handleCommand(const std::string& cmd) {
    if (cmd == CMD_START) {
        startService();
    } else if (cmd == CMD_STOP) {
        stopService();
    } else if (cmd == CMD_RESTART) {
        restartService();
    }
    auto dbUrl = config[CONFIG_DATABASE_URL];
    if (status == STATUS_ACTIVE) {
        activate();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `string_literal` in `binary_expression` (equality checks with `==`, `!=`) or `subscript_argument_list` (map/array access)
- **Detection approach**: Find `string_literal` nodes used in equality comparisons (`==`, `!=`) or as keys in `subscript_expression` via `subscript_argument_list`. Exclude logging strings, format strings, stream insertion (`<<`) operands used for output, and header include paths. Flag repeated identical strings across a function or file.
- **S-expression query sketch**:
```scheme
(binary_expression
  operator: ["==" "!="]
  [left: (string_literal) @string_lit
   right: (string_literal) @string_lit])

(subscript_expression
  (subscript_argument_list
    (string_literal) @string_lit))
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
