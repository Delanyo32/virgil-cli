# Legacy API Usage -- Java

## Overview
Legacy API usage in Java refers to relying on older patterns or language constructs when more maintainable, type-safe, or performant alternatives exist. Common examples include long `instanceof` chains instead of polymorphic dispatch, and raw string concatenation with `+` inside loops instead of `StringBuilder`.

## Why It's a Tech Debt Concern
Chains of `instanceof` checks violate the Open/Closed Principle -- adding a new subtype requires modifying every `instanceof` chain rather than adding a new class with the appropriate method override. Missing a chain during modification causes silent logic errors. String concatenation with `+` inside loops creates a new `String` object on every iteration because `String` is immutable in Java, leading to O(n^2) memory allocation and copying in tight loops. Both patterns are easy to introduce and hard to notice during review.

## Applicability
- **Relevance**: high (both patterns are pervasive in enterprise Java codebases)
- **Languages covered**: `.java`
- **Frameworks/libraries**: N/A (language-level patterns)

---

## Pattern 1: instanceof Chains Instead of Polymorphism

### Description
Using a series of `instanceof` checks followed by casts to determine behavior based on an object's runtime type. This bypasses Java's type system and polymorphic dispatch, scattering type-dependent logic across the codebase instead of encapsulating it in each type's own methods.

### Bad Code (Anti-pattern)
```java
public class ShapeRenderer {
    public double calculateArea(Shape shape) {
        if (shape instanceof Circle) {
            Circle c = (Circle) shape;
            return Math.PI * c.getRadius() * c.getRadius();
        } else if (shape instanceof Rectangle) {
            Rectangle r = (Rectangle) shape;
            return r.getWidth() * r.getHeight();
        } else if (shape instanceof Triangle) {
            Triangle t = (Triangle) shape;
            return 0.5 * t.getBase() * t.getHeight();
        } else if (shape instanceof Polygon) {
            Polygon p = (Polygon) shape;
            return p.getVertices().stream()
                .mapToDouble(v -> v.x * v.y)
                .sum();
        }
        throw new IllegalArgumentException("Unknown shape: " + shape.getClass());
    }

    public String describe(Shape shape) {
        if (shape instanceof Circle) {
            return "A circle with radius " + ((Circle) shape).getRadius();
        } else if (shape instanceof Rectangle) {
            return "A rectangle " + ((Rectangle) shape).getWidth() + "x" + ((Rectangle) shape).getHeight();
        } else if (shape instanceof Triangle) {
            return "A triangle with base " + ((Triangle) shape).getBase();
        }
        return "Unknown shape";
    }
}
```

### Good Code (Fix)
```java
public abstract class Shape {
    public abstract double area();
    public abstract String describe();
}

public class Circle extends Shape {
    private final double radius;

    public Circle(double radius) { this.radius = radius; }

    @Override
    public double area() {
        return Math.PI * radius * radius;
    }

    @Override
    public String describe() {
        return "A circle with radius " + radius;
    }
}

public class Rectangle extends Shape {
    private final double width;
    private final double height;

    public Rectangle(double width, double height) {
        this.width = width;
        this.height = height;
    }

    @Override
    public double area() {
        return width * height;
    }

    @Override
    public String describe() {
        return "A rectangle " + width + "x" + height;
    }
}

public class ShapeRenderer {
    public double calculateArea(Shape shape) {
        return shape.area();
    }

    public String describe(Shape shape) {
        return shape.describe();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `instanceof_expression`, `if_statement`, `binary_expression`
- **Detection approach**: Find `instanceof_expression` nodes. Count consecutive `if`/`else if` branches where each condition contains an `instanceof_expression` on the same variable. Flag when 3+ branches form an `instanceof` chain -- this strongly indicates a missing polymorphic method. Also flag when the `instanceof` is immediately followed by a cast expression to the same type.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (parenthesized_expression
    (instanceof_expression
      left: (identifier) @checked_var
      right: (type_identifier) @checked_type)))

(instanceof_expression
  left: (identifier) @var
  right: (type_identifier) @type)
```

### Pipeline Mapping
- **Pipeline name**: `instanceof_chains`
- **Pattern name**: `instanceof_chain`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: String Concatenation in Loops

### Description
Using the `+` operator to concatenate strings inside `for`, `while`, or `do-while` loops. Because `String` is immutable in Java, each `+` concatenation creates a new `String` object, copies all previous characters, and discards the old object. In a loop of N iterations, this produces O(N^2) time and memory behavior.

### Bad Code (Anti-pattern)
```java
public class ReportGenerator {
    public String generateCsv(List<Record> records) {
        String csv = "id,name,email,status\n";
        for (Record record : records) {
            csv = csv + record.getId() + ","
                + record.getName() + ","
                + record.getEmail() + ","
                + record.getStatus() + "\n";
        }
        return csv;
    }

    public String buildQuery(List<String> conditions) {
        String query = "SELECT * FROM users WHERE ";
        for (int i = 0; i < conditions.size(); i++) {
            if (i > 0) {
                query = query + " AND ";
            }
            query = query + conditions.get(i);
        }
        return query;
    }

    public String formatList(List<String> items) {
        String result = "";
        for (String item : items) {
            result += "- " + item + "\n";
        }
        return result;
    }
}
```

### Good Code (Fix)
```java
public class ReportGenerator {
    public String generateCsv(List<Record> records) {
        StringBuilder csv = new StringBuilder("id,name,email,status\n");
        for (Record record : records) {
            csv.append(record.getId()).append(',')
               .append(record.getName()).append(',')
               .append(record.getEmail()).append(',')
               .append(record.getStatus()).append('\n');
        }
        return csv.toString();
    }

    public String buildQuery(List<String> conditions) {
        StringBuilder query = new StringBuilder("SELECT * FROM users WHERE ");
        for (int i = 0; i < conditions.size(); i++) {
            if (i > 0) {
                query.append(" AND ");
            }
            query.append(conditions.get(i));
        }
        return query.toString();
    }

    public String formatList(List<String> items) {
        // Or use String.join / Collectors.joining for simpler cases
        return items.stream()
            .map(item -> "- " + item)
            .collect(Collectors.joining("\n"));
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment_expression` with `+` or `+=` operator and `string` type, inside `for_statement`, `enhanced_for_statement`, `while_statement`, `do_statement`
- **Detection approach**: Find `assignment_expression` nodes inside loop bodies where the operator is `+=` and the right-hand side involves string concatenation, or where the left-hand side variable appears on the right side with a `+` operator (e.g., `s = s + "..."`). The variable must be of type `String` (check the declaration). Flag all occurrences inside loops.
- **S-expression query sketch**:
```scheme
(enhanced_for_statement
  body: (block
    (expression_statement
      (assignment_expression
        left: (identifier) @var
        right: (binary_expression
          operator: "+")))))

(for_statement
  body: (block
    (expression_statement
      (assignment_expression
        left: (identifier) @var
        operator: "+="
        right: (_) @concat_value))))
```

### Pipeline Mapping
- **Pipeline name**: `string_concat_in_loops`
- **Pattern name**: `loop_string_concatenation`
- **Severity**: warning
- **Confidence**: high
