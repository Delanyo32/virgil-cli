# Comment Ratio -- C#

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .cs
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```csharp
public ValidationResult ValidateDocument(Document doc, RuleSet rules)
{
    var errors = new List<ValidationError>();
    foreach (var rule in rules.Rules)
    {
        if (!rule.IsEnabled)
            continue;

        var fields = doc.GetFields(rule.TargetSection);
        foreach (var field in fields)
        {
            if (rule.Type == RuleType.Required && string.IsNullOrWhiteSpace(field.Value))
            {
                errors.Add(new ValidationError(field.Name, rule.Message, Severity.Error));
            }
            else if (rule.Type == RuleType.Pattern)
            {
                if (!Regex.IsMatch(field.Value ?? "", rule.Pattern))
                {
                    errors.Add(new ValidationError(field.Name, rule.Message, Severity.Warning));
                }
            }
            else if (rule.Type == RuleType.CrossField)
            {
                var otherField = fields.FirstOrDefault(f => f.Name == rule.DependsOn);
                if (otherField != null && !string.IsNullOrEmpty(otherField.Value))
                {
                    if (string.IsNullOrWhiteSpace(field.Value))
                    {
                        errors.Add(new ValidationError(field.Name, rule.Message, Severity.Error));
                    }
                }
            }
            else if (rule.Type == RuleType.Range)
            {
                if (decimal.TryParse(field.Value, out var val))
                {
                    if (val < rule.Min || val > rule.Max)
                        errors.Add(new ValidationError(field.Name, rule.Message, Severity.Warning));
                }
                else
                {
                    errors.Add(new ValidationError(field.Name, "Not a number", Severity.Error));
                }
            }
        }
    }
    return new ValidationResult(errors.Count == 0, errors);
}
```

### Good Code (Fix)
```csharp
/// <summary>
/// Validates a document against a rule set. Rules are evaluated per-field within
/// their target section. Cross-field rules enforce conditional requirements
/// (e.g., "if field A is filled, field B is mandatory").
/// </summary>
public ValidationResult ValidateDocument(Document doc, RuleSet rules)
{
    var errors = new List<ValidationError>();
    foreach (var rule in rules.Rules)
    {
        if (!rule.IsEnabled)
            continue;

        var fields = doc.GetFields(rule.TargetSection);
        foreach (var field in fields)
        {
            if (rule.Type == RuleType.Required && string.IsNullOrWhiteSpace(field.Value))
            {
                errors.Add(new ValidationError(field.Name, rule.Message, Severity.Error));
            }
            else if (rule.Type == RuleType.Pattern)
            {
                if (!Regex.IsMatch(field.Value ?? "", rule.Pattern))
                {
                    errors.Add(new ValidationError(field.Name, rule.Message, Severity.Warning));
                }
            }
            else if (rule.Type == RuleType.CrossField)
            {
                // Cross-field: only enforce this field when the dependency field
                // has a value -- avoids false positives on optional groups
                var otherField = fields.FirstOrDefault(f => f.Name == rule.DependsOn);
                if (otherField != null && !string.IsNullOrEmpty(otherField.Value))
                {
                    if (string.IsNullOrWhiteSpace(field.Value))
                    {
                        errors.Add(new ValidationError(field.Name, rule.Message, Severity.Error));
                    }
                }
            }
            else if (rule.Type == RuleType.Range)
            {
                // Parse failure is an error, not a warning -- the field must be
                // numeric for range validation to be meaningful
                if (decimal.TryParse(field.Value, out var val))
                {
                    if (val < rule.Min || val > rule.Max)
                        errors.Add(new ValidationError(field.Name, rule.Message, Severity.Warning));
                }
                else
                {
                    errors.Add(new ValidationError(field.Name, "Not a number", Severity.Error));
                }
            }
        }
    }
    return new ValidationResult(errors.Count == 0, errors);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `constructor_declaration` for function bodies; `comment` for `//` and `/* */`; XML doc comments `///` appear as `comment` nodes
- **Detection approach**: Count comment lines and code lines within a method body. Calculate ratio. Flag methods with CC > 5 and comment ratio below threshold. Consider `///` XML doc comments above the method signature as part of the method's documentation.
- **S-expression query sketch**:
  ```scheme
  ;; Capture method body and any comments within it
  (method_declaration
    body: (block) @function.body)

  (constructor_declaration
    body: (block) @function.body)

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
```csharp
public void SendNotification(User user, string message)
{
    // Check if user is null
    if (user == null)
    {
        // Throw argument null exception
        throw new ArgumentNullException(nameof(user));
    }

    // Get the email address
    var email = user.Email;

    // Check if email is not null or empty
    if (!string.IsNullOrEmpty(email))
    {
        // Create new email message
        var emailMessage = new EmailMessage(email, message);

        // Send the email
        _emailService.Send(emailMessage);
    }

    // Log the notification
    _logger.LogInformation("Notification sent to {UserId}", user.Id);
}
```

### Good Code (Fix)
```csharp
public void SendNotification(User user, string message)
{
    if (user == null)
        throw new ArgumentNullException(nameof(user));

    var email = user.Email;
    if (!string.IsNullOrEmpty(email))
    {
        var emailMessage = new EmailMessage(email, message);
        _emailService.Send(emailMessage);
    }

    // Log even when email is missing -- audit trail for compliance
    _logger.LogInformation("Notification sent to {UserId}", user.Id);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment` adjacent to `local_declaration_statement`, `expression_statement`, `return_statement`, `if_statement`, `throw_statement`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (block
    (comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
