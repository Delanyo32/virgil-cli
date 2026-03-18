# Duplicate Code -- C++

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more functions or method definitions with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared function with parameters or a template. Common in duplicate template specializations.

### Bad Code (Anti-pattern)
```cpp
class DataProcessor {
public:
    std::vector<UserResult> processUserRecords(const std::vector<UserRecord>& records) {
        std::vector<UserResult> results;
        results.reserve(records.size());
        for (const auto& record : records) {
            if (record.name.empty() || record.email.empty()) {
                std::cerr << "Skipping invalid user record: " << record.id << "\n";
                continue;
            }
            std::string normalized = record.name;
            std::transform(normalized.begin(), normalized.end(), normalized.begin(), ::tolower);
            normalized.erase(0, normalized.find_first_not_of(' '));
            normalized.erase(normalized.find_last_not_of(' ') + 1);
            auto amount = record.quantity * record.price;
            auto tax = amount * 0.08;
            results.push_back({record.id, normalized, record.email, amount, tax});
        }
        std::sort(results.begin(), results.end(),
                  [](const auto& a, const auto& b) { return a.amount > b.amount; });
        return results;
    }

    std::vector<VendorResult> processVendorRecords(const std::vector<VendorRecord>& records) {
        std::vector<VendorResult> results;
        results.reserve(records.size());
        for (const auto& record : records) {
            if (record.name.empty() || record.email.empty()) {
                std::cerr << "Skipping invalid vendor record: " << record.id << "\n";
                continue;
            }
            std::string normalized = record.name;
            std::transform(normalized.begin(), normalized.end(), normalized.begin(), ::tolower);
            normalized.erase(0, normalized.find_first_not_of(' '));
            normalized.erase(normalized.find_last_not_of(' ') + 1);
            auto amount = record.quantity * record.price;
            auto tax = amount * 0.08;
            results.push_back({record.id, normalized, record.email, amount, tax});
        }
        std::sort(results.begin(), results.end(),
                  [](const auto& a, const auto& b) { return a.amount > b.amount; });
        return results;
    }
};
```

### Good Code (Fix)
```cpp
class DataProcessor {
public:
    template <typename Record, typename Result>
    std::vector<Result> processRecords(const std::vector<Record>& records, const std::string& entityType) {
        std::vector<Result> results;
        results.reserve(records.size());
        for (const auto& record : records) {
            if (record.name.empty() || record.email.empty()) {
                std::cerr << "Skipping invalid " << entityType << " record: " << record.id << "\n";
                continue;
            }
            std::string normalized = record.name;
            std::transform(normalized.begin(), normalized.end(), normalized.begin(), ::tolower);
            normalized.erase(0, normalized.find_first_not_of(' '));
            normalized.erase(normalized.find_last_not_of(' ') + 1);
            auto amount = record.quantity * record.price;
            auto tax = amount * 0.08;
            results.push_back({record.id, normalized, record.email, amount, tax});
        }
        std::sort(results.begin(), results.end(),
                  [](const auto& a, const auto& b) { return a.amount > b.amount; });
        return results;
    }

    auto processUserRecords(const std::vector<UserRecord>& records) {
        return processRecords<UserRecord, UserResult>(records, "user");
    }

    auto processVendorRecords(const std::vector<VendorRecord>& records) {
        return processRecords<VendorRecord, VendorResult>(records, "vendor");
    }
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `compound_statement`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(function_definition
  declarator: (function_declarator
    declarator: (_) @func_name)
  body: (compound_statement) @func_body)
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `cloned_function_bodies`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Repeated Logic Blocks Within a Function

### Description
The same sequence of 5+ statements repeated within a function or across methods in the same class, often due to copy-paste during development. Common in duplicate template specializations and repeated STL algorithm chains.

### Bad Code (Anti-pattern)
```cpp
void generateReports(const Database& db) {
    // Sales report
    auto salesData = db.query<SaleRecord>("SELECT * FROM sales WHERE year = 2024");
    std::vector<SaleRecord> validSales;
    std::copy_if(salesData.begin(), salesData.end(), std::back_inserter(validSales),
                 [](const auto& r) { return r.amount > 0; });
    std::sort(validSales.begin(), validSales.end(),
              [](const auto& a, const auto& b) { return a.date < b.date; });
    double salesTotal = std::accumulate(validSales.begin(), validSales.end(), 0.0,
                                        [](double sum, const auto& r) { return sum + r.amount; });
    double salesAvg = validSales.empty() ? 0.0 : salesTotal / validSales.size();
    std::cout << "Sales: total=" << salesTotal << " avg=" << salesAvg
              << " count=" << validSales.size() << "\n";

    // Returns report
    auto returnsData = db.query<ReturnRecord>("SELECT * FROM returns WHERE year = 2024");
    std::vector<ReturnRecord> validReturns;
    std::copy_if(returnsData.begin(), returnsData.end(), std::back_inserter(validReturns),
                 [](const auto& r) { return r.amount > 0; });
    std::sort(validReturns.begin(), validReturns.end(),
              [](const auto& a, const auto& b) { return a.date < b.date; });
    double returnsTotal = std::accumulate(validReturns.begin(), validReturns.end(), 0.0,
                                           [](double sum, const auto& r) { return sum + r.amount; });
    double returnsAvg = validReturns.empty() ? 0.0 : returnsTotal / validReturns.size();
    std::cout << "Returns: total=" << returnsTotal << " avg=" << returnsAvg
              << " count=" << validReturns.size() << "\n";
}
```

### Good Code (Fix)
```cpp
struct ReportSummary {
    double total;
    double average;
    size_t count;
};

template <typename Record>
ReportSummary summarize(const std::string& query, const Database& db) {
    auto data = db.query<Record>(query);
    std::vector<Record> valid;
    std::copy_if(data.begin(), data.end(), std::back_inserter(valid),
                 [](const auto& r) { return r.amount > 0; });
    std::sort(valid.begin(), valid.end(),
              [](const auto& a, const auto& b) { return a.date < b.date; });
    double total = std::accumulate(valid.begin(), valid.end(), 0.0,
                                    [](double sum, const auto& r) { return sum + r.amount; });
    double avg = valid.empty() ? 0.0 : total / valid.size();
    return {total, avg, valid.size()};
}

void generateReports(const Database& db) {
    auto sales = summarize<SaleRecord>("SELECT * FROM sales WHERE year = 2024", db);
    std::cout << "Sales: total=" << sales.total << " avg=" << sales.average
              << " count=" << sales.count << "\n";

    auto returns = summarize<ReturnRecord>("SELECT * FROM returns WHERE year = 2024", db);
    std::cout << "Returns: total=" << returns.total << " avg=" << returns.average
              << " count=" << returns.count << "\n";
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `compound_statement`, `declaration`, `expression_statement`, `if_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across function bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences.
- **S-expression query sketch**:
```scheme
(compound_statement
  (_) @stmt)

(function_definition
  body: (compound_statement
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
