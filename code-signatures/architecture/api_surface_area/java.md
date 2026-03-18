# API Surface Area -- Java

## Overview
API surface area in Java is controlled through access modifiers: `public`, `protected`, `private`, and package-private (default). A well-designed class exposes only the methods necessary for its consumers while keeping internal state and helper logic private. Tracking the ratio of public to total members per class identifies types that over-expose their internals, increasing coupling between packages and making library evolution difficult.

## Why It's an Architecture Concern
In Java, `public` classes and members form a binding contract, especially in libraries distributed as JARs. Every public method is a commitment that downstream consumers may depend on, making removal or signature changes a breaking modification. Excessive public APIs bloat Javadoc, confuse consumers with too many entry points, and create hidden coupling across module boundaries. When classes expose fields directly instead of using encapsulated getters with defensive copies, internal data structures leak to callers, preventing safe changes to representation. A narrow public surface with `private` internals preserves encapsulation and supports safe evolution.

## Applicability
- **Relevance**: high
- **Languages covered**: `.java`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```java
public class ReportGenerator {
    public void loadData(String path) { }
    public void validateData() { }
    public void cleanData() { }
    public void aggregateByRegion() { }
    public void aggregateByDate() { }
    public void calculateTotals() { }
    public void formatAsHtml() { }
    public void formatAsPdf() { }
    public void formatAsCsv() { }
    public void writeToFile(String path) { }
    public void sendByEmail(String address) { }
    public void archiveReport(String archivePath) { }
}
```

### Good Code (Fix)
```java
public class ReportGenerator {
    public Report generate(String dataPath) { return null; }
    public void export(Report report, Format format, String outputPath) { }
    public void send(Report report, String emailAddress) { }

    private void loadData(String path) { }
    private void validateData() { }
    private void cleanData() { }
    private void aggregateByRegion() { }
    private void aggregateByDate() { }
    private void calculateTotals() { }
    private String formatReport(Format format) { return ""; }
    private void archiveReport(String archivePath) { }
    private void writeToFile(String path, String content) { }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `field_declaration` with `modifiers` containing access keywords
- **Detection approach**: Count all method and field declarations in a class. A member is exported if its `modifiers` node contains a `public` marker. Flag classes where total members >= 10 and public/total > 0.8.
- **S-expression query sketch**:
```scheme
;; Match public methods inside class declarations
(class_declaration
  name: (identifier) @class.name
  body: (class_body
    (method_declaration
      (modifiers) @mods
      name: (identifier) @method.name)))

;; Post-process: check if @mods text contains "public"

;; Match all methods for total count
(class_declaration
  body: (class_body
    (method_declaration
      name: (identifier) @all.method.name)))
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```java
public class SessionManager {
    public HashMap<String, Session> activeSessions;
    public ArrayList<String> sessionIds;
    public ConcurrentLinkedQueue<Event> eventQueue;
    public int maxSessions;
    public long timeoutMs;
    public boolean debugMode;

    public Session getSession(String id) { return null; }
    public void createSession(String userId) { }
}
```

### Good Code (Fix)
```java
public class SessionManager {
    private final Map<String, Session> activeSessions;
    private final List<String> sessionIds;
    private final Queue<Event> eventQueue;
    private int maxSessions;
    private long timeoutMs;

    public SessionManager(SessionConfig config) {
        this.activeSessions = new HashMap<>();
        this.sessionIds = new ArrayList<>();
        this.eventQueue = new ConcurrentLinkedQueue<>();
    }

    public Session getSession(String id) { return null; }
    public void createSession(String userId) { }
    public int activeCount() { return activeSessions.size(); }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration` with `modifiers` containing `public`, inside `class_body`
- **Detection approach**: Find public classes and check for public field declarations. Public fields, especially with concrete collection types (e.g., `HashMap`, `ArrayList`), indicate leaked implementation. Flag classes with 2+ public non-static-final fields.
- **S-expression query sketch**:
```scheme
;; Match public fields in class bodies
(class_declaration
  (modifiers) @class.mods
  name: (identifier) @class.name
  body: (class_body
    (field_declaration
      (modifiers) @field.mods
      type: (_) @field.type
      declarator: (variable_declarator
        name: (identifier) @field.name))))

;; Post-process: check @field.mods contains "public", @field.type is concrete
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
