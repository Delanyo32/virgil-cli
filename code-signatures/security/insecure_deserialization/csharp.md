# Insecure Deserialization -- C#

## Overview
Insecure deserialization in C# involves formatters and serializers that can instantiate arbitrary types during deserialization. `BinaryFormatter` is the most notorious vector (Microsoft has formally deprecated it), and `Newtonsoft.Json` with `TypeNameHandling.All` allows type confusion attacks via `$type` metadata in JSON payloads.

## Why It's a Security Concern
`BinaryFormatter.Deserialize()` can instantiate any type in loaded assemblies, enabling remote code execution via gadget chains (similar to Java's `ObjectInputStream`). `Newtonsoft.Json` with `TypeNameHandling` set to `All`, `Auto`, or `Objects` reads `$type` annotations from JSON, allowing attackers to instantiate dangerous types like `System.Diagnostics.Process` or `System.IO.FileInfo`.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: System.Runtime.Serialization.Formatters.Binary.BinaryFormatter, Newtonsoft.Json, System.Web.Script.Serialization.JavaScriptSerializer, System.Runtime.Serialization.NetDataContractSerializer

---

## Pattern 1: BinaryFormatter.Deserialize() on Untrusted Data

### Description
Using `BinaryFormatter.Deserialize()` on data from untrusted sources. Microsoft has officially deprecated `BinaryFormatter` as of .NET 8 due to its inherent insecurity. It can instantiate arbitrary types present in loaded assemblies, enabling RCE.

### Bad Code (Anti-pattern)
```csharp
public object DeserializeData(byte[] data)
{
    using var stream = new MemoryStream(data);
    var formatter = new BinaryFormatter();
    return formatter.Deserialize(stream); // RCE: attacker controls type instantiation
}
```

### Good Code (Fix)
```csharp
public UserData DeserializeData(byte[] data)
{
    var json = Encoding.UTF8.GetString(data);
    return JsonSerializer.Deserialize<UserData>(json); // System.Text.Json, type-safe
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `object_creation_expression`, `identifier`
- **Detection approach**: Find `object_creation_expression` nodes constructing `BinaryFormatter`, or `invocation_expression` nodes calling `.Deserialize()` on a variable of type `BinaryFormatter`. Flag any instantiation of `BinaryFormatter` as a security concern.
- **S-expression query sketch**:
```scheme
(invocation_expression
  function: (member_access_expression
    expression: (identifier) @target
    name: (identifier) @method)
  arguments: (argument_list (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `binary_formatter`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Newtonsoft.Json with TypeNameHandling.All

### Description
Configuring `Newtonsoft.Json` (Json.NET) with `TypeNameHandling.All`, `TypeNameHandling.Auto`, or `TypeNameHandling.Objects`. This causes the deserializer to read `$type` metadata from JSON, allowing attackers to specify which .NET types to instantiate — enabling type confusion and RCE.

### Bad Code (Anti-pattern)
```csharp
public object ParsePayload(string json)
{
    var settings = new JsonSerializerSettings
    {
        TypeNameHandling = TypeNameHandling.All // RCE: attacker controls $type
    };
    return JsonConvert.DeserializeObject(json, settings);
}
```

### Good Code (Fix)
```csharp
public UserPayload ParsePayload(string json)
{
    // Use System.Text.Json (no type name handling) or Newtonsoft with TypeNameHandling.None
    return JsonConvert.DeserializeObject<UserPayload>(json);
    // TypeNameHandling defaults to None — safe
}

// If TypeNameHandling is needed, use a SerializationBinder to restrict types:
public object ParsePayloadSafe(string json)
{
    var settings = new JsonSerializerSettings
    {
        TypeNameHandling = TypeNameHandling.Auto,
        SerializationBinder = new AllowListBinder(typeof(UserPayload), typeof(Address))
    };
    return JsonConvert.DeserializeObject(json, settings);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment_expression`, `member_access_expression`, `identifier`, `object_creation_expression`
- **Detection approach**: Find `assignment_expression` or `initializer_expression` nodes where the left side is `TypeNameHandling` (a property of `JsonSerializerSettings`) and the right side is `TypeNameHandling.All`, `TypeNameHandling.Auto`, or `TypeNameHandling.Objects`. Also detect `JsonSerializerSettings` construction with inline property initializer setting `TypeNameHandling`.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (member_access_expression
    name: (identifier) @property)
  right: (member_access_expression
    expression: (identifier) @enum_type
    name: (identifier) @enum_value))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `newtonsoft_type_name_handling`
- **Severity**: error
- **Confidence**: high
