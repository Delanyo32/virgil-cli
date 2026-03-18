# Function Length -- C++

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Threshold**: 50 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 50-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```cpp
std::optional<Report> ReportGenerator::generate(const std::string& input_path, const Config& config)
{
    // Validate input
    if (input_path.empty()) {
        logger_.error("Input path is empty");
        return std::nullopt;
    }
    std::ifstream file(input_path);
    if (!file.is_open()) {
        logger_.error("Cannot open file: {}", input_path);
        return std::nullopt;
    }
    auto extension = std::filesystem::path(input_path).extension().string();
    if (extension != ".csv" && extension != ".tsv") {
        logger_.error("Unsupported format: {}", extension);
        return std::nullopt;
    }

    // Parse records
    char delimiter = (extension == ".tsv") ? '\t' : ',';
    std::string header_line;
    std::getline(file, header_line);
    auto headers = split(header_line, delimiter);
    if (headers.size() < 4) {
        logger_.error("Insufficient columns in header");
        return std::nullopt;
    }

    std::vector<Record> records;
    std::vector<std::string> errors;
    std::string line;
    int line_num = 1;
    while (std::getline(file, line)) {
        line_num++;
        auto fields = split(line, delimiter);
        if (fields.size() != headers.size()) {
            errors.push_back(fmt::format("Line {}: field count mismatch", line_num));
            continue;
        }
        try {
            Record r;
            r.id = std::stoi(fields[0]);
            r.name = trim(fields[1]);
            r.amount = std::stod(fields[2]);
            r.date = parse_date(fields[3]);
            if (r.amount < 0 && !config.allow_negative) {
                errors.push_back(fmt::format("Line {}: negative amount", line_num));
                continue;
            }
            records.push_back(std::move(r));
        } catch (const std::exception& e) {
            errors.push_back(fmt::format("Line {}: parse error: {}", line_num, e.what()));
        }
    }

    if (records.empty()) {
        logger_.error("No valid records found");
        return std::nullopt;
    }

    // Compute statistics
    double total = 0.0;
    double min_amount = std::numeric_limits<double>::max();
    double max_amount = std::numeric_limits<double>::lowest();
    std::unordered_map<std::string, std::vector<const Record*>> by_month;
    for (const auto& r : records) {
        total += r.amount;
        min_amount = std::min(min_amount, r.amount);
        max_amount = std::max(max_amount, r.amount);
        auto month_key = fmt::format("{:%Y-%m}", r.date);
        by_month[month_key].push_back(&r);
    }
    double average = total / static_cast<double>(records.size());

    // Build report
    Report report;
    report.source_file = input_path;
    report.record_count = records.size();
    report.error_count = errors.size();
    report.total = total;
    report.average = average;
    report.min_amount = min_amount;
    report.max_amount = max_amount;
    for (const auto& [month, items] : by_month) {
        double month_total = 0.0;
        for (const auto* item : items) {
            month_total += item->amount;
        }
        report.monthly_totals[month] = month_total;
    }

    // Write output
    auto output_path = std::filesystem::path(config.output_dir) / "report.json";
    std::ofstream out(output_path);
    if (!out.is_open()) {
        logger_.error("Cannot write to: {}", output_path.string());
        return std::nullopt;
    }
    out << report.to_json();
    out.close();

    if (!errors.empty()) {
        auto error_path = std::filesystem::path(config.output_dir) / "errors.log";
        std::ofstream err_out(error_path);
        for (const auto& e : errors) {
            err_out << e << '\n';
        }
    }

    return report;
}
```

### Good Code (Fix)
```cpp
std::optional<std::ifstream> ReportGenerator::validate_and_open(const std::string& input_path)
{
    if (input_path.empty()) {
        logger_.error("Input path is empty");
        return std::nullopt;
    }
    std::ifstream file(input_path);
    if (!file.is_open()) {
        logger_.error("Cannot open file: {}", input_path);
        return std::nullopt;
    }
    auto ext = std::filesystem::path(input_path).extension().string();
    if (ext != ".csv" && ext != ".tsv") {
        logger_.error("Unsupported format: {}", ext);
        return std::nullopt;
    }
    return file;
}

ParseResult ReportGenerator::parse_records(std::ifstream& file, char delimiter, const Config& config)
{
    std::string header_line;
    std::getline(file, header_line);
    auto headers = split(header_line, delimiter);

    ParseResult result;
    std::string line;
    int line_num = 1;
    while (std::getline(file, line)) {
        line_num++;
        auto parsed = parse_single_record(line, delimiter, headers.size(), line_num, config);
        if (parsed.has_value())
            result.records.push_back(std::move(*parsed));
        else
            result.errors.push_back(parsed.error());
    }
    return result;
}

Statistics ReportGenerator::compute_statistics(const std::vector<Record>& records)
{
    Statistics stats;
    for (const auto& r : records) {
        stats.total += r.amount;
        stats.min_amount = std::min(stats.min_amount, r.amount);
        stats.max_amount = std::max(stats.max_amount, r.amount);
        stats.monthly_totals[fmt::format("{:%Y-%m}", r.date)] += r.amount;
    }
    stats.average = stats.total / static_cast<double>(records.size());
    return stats;
}

std::optional<Report> ReportGenerator::generate(const std::string& input_path, const Config& config)
{
    auto file = validate_and_open(input_path);
    if (!file) return std::nullopt;

    char delim = (std::filesystem::path(input_path).extension() == ".tsv") ? '\t' : ',';
    auto [records, errors] = parse_records(*file, delim, config);
    if (records.empty()) {
        logger_.error("No valid records found");
        return std::nullopt;
    }

    auto stats = compute_statistics(records);
    Report report = build_report(input_path, records.size(), errors.size(), stats);
    write_output(report, errors, config.output_dir);
    return report;
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

  (function_definition
    declarator: (function_declarator
      declarator: (qualified_identifier) @func.name)
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
A single `main()` function, event callback, or request handler method that contains all program logic inline -- initialization, configuration, processing loop, cleanup -- instead of delegating to focused functions or classes.

### Bad Code (Anti-pattern)
```cpp
int main(int argc, char* argv[])
{
    if (argc < 2) {
        std::cerr << "Usage: " << argv[0] << " <config-file>\n";
        return 1;
    }

    // Parse config
    std::ifstream config_file(argv[1]);
    if (!config_file.is_open()) {
        std::cerr << "Cannot open config: " << argv[1] << '\n';
        return 1;
    }
    nlohmann::json config;
    try {
        config_file >> config;
    } catch (const nlohmann::json::exception& e) {
        std::cerr << "Invalid config JSON: " << e.what() << '\n';
        return 1;
    }
    auto host = config.value("host", "0.0.0.0");
    auto port = config.value("port", 8080);
    auto db_url = config.value("database_url", "");
    if (db_url.empty()) {
        std::cerr << "database_url is required in config\n";
        return 1;
    }

    // Initialize database
    auto db = std::make_unique<Database>(db_url);
    if (!db->connect()) {
        std::cerr << "Failed to connect to database\n";
        return 1;
    }
    if (!db->migrate()) {
        std::cerr << "Database migration failed\n";
        return 1;
    }

    // Set up HTTP server
    httplib::Server server;
    server.Post("/api/data", [&db](const httplib::Request& req, httplib::Response& res) {
        auto body = nlohmann::json::parse(req.body, nullptr, false);
        if (body.is_discarded()) {
            res.status = 400;
            res.set_content(R"({"error":"Invalid JSON"})", "application/json");
            return;
        }
        auto result = db->insert(body);
        if (result.has_error()) {
            res.status = 500;
            res.set_content(R"({"error":"Database error"})", "application/json");
            return;
        }
        res.status = 201;
        res.set_content(result.to_json(), "application/json");
    });
    server.Get("/api/health", [&db](const httplib::Request&, httplib::Response& res) {
        auto healthy = db->ping();
        res.status = healthy ? 200 : 503;
        res.set_content(healthy ? R"({"status":"ok"})" : R"({"status":"unhealthy"})", "application/json");
    });

    std::cout << "Starting server on " << host << ":" << port << '\n';
    if (!server.listen(host.c_str(), port)) {
        std::cerr << "Failed to start server\n";
        return 1;
    }

    return 0;
}
```

### Good Code (Fix)
```cpp
Config parse_config(const std::string& path)
{
    std::ifstream file(path);
    if (!file.is_open())
        throw std::runtime_error("Cannot open config: " + path);
    nlohmann::json j;
    file >> j;
    return Config::from_json(j);
}

std::unique_ptr<Database> init_database(const std::string& url)
{
    auto db = std::make_unique<Database>(url);
    if (!db->connect()) throw std::runtime_error("Database connection failed");
    if (!db->migrate()) throw std::runtime_error("Database migration failed");
    return db;
}

void register_routes(httplib::Server& server, Database& db)
{
    server.Post("/api/data", [&db](const httplib::Request& req, httplib::Response& res) {
        handlers::create_data(req, res, db);
    });
    server.Get("/api/health", [&db](const httplib::Request& req, httplib::Response& res) {
        handlers::health_check(req, res, db);
    });
}

int main(int argc, char* argv[])
{
    if (argc < 2) {
        std::cerr << "Usage: " << argv[0] << " <config-file>\n";
        return 1;
    }
    try {
        auto config = parse_config(argv[1]);
        auto db = init_database(config.database_url);

        httplib::Server server;
        register_routes(server, *db);

        std::cout << "Starting on " << config.host << ":" << config.port << '\n';
        server.listen(config.host.c_str(), config.port);
    } catch (const std::exception& e) {
        std::cerr << "Fatal: " << e.what() << '\n';
        return 1;
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 50. The `main()` function and callback lambdas are detected the same way as regular functions; the pattern name differentiates them in reporting.
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
