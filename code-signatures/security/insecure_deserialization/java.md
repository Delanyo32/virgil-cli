# Insecure Deserialization -- Java

## Overview
Insecure deserialization is one of the most critical vulnerability classes in Java. The native `ObjectInputStream.readObject()` mechanism can instantiate arbitrary classes present on the classpath, enabling remote code execution via gadget chains. This is consistently listed in the OWASP Top 10 and has led to major breaches (Apache Struts, Jenkins, WebLogic).

## Why It's a Security Concern
Java's native serialization (`ObjectInputStream`) invokes constructors and `readObject()` methods of the serialized class during deserialization. If an attacker controls the serialized byte stream, they can chain together "gadget" classes already on the classpath (e.g., from Commons Collections, Spring, or Hibernate) to achieve remote code execution. `XMLDecoder` similarly instantiates arbitrary objects from XML descriptions.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: java.io.ObjectInputStream, java.beans.XMLDecoder, Apache Commons Collections, Spring Framework, Hibernate, WebLogic, JBoss

---

## Pattern 1: ObjectInputStream.readObject() on Untrusted Data

### Description
Creating an `ObjectInputStream` from an untrusted source (network socket, HTTP request body, file upload, message queue) and calling `readObject()`. This is the classic Java deserialization RCE vector — any class on the classpath with a compatible `readObject()` method becomes a potential gadget.

### Bad Code (Anti-pattern)
```java
public Object handleRequest(HttpServletRequest request) throws Exception {
    ObjectInputStream ois = new ObjectInputStream(request.getInputStream());
    Object data = ois.readObject(); // RCE: attacker sends crafted serialized object
    ois.close();
    return data;
}
```

### Good Code (Fix)
```java
public UserData handleRequest(HttpServletRequest request) throws Exception {
    // Use JSON instead of Java serialization
    ObjectMapper mapper = new ObjectMapper();
    UserData data = mapper.readValue(request.getInputStream(), UserData.class);
    return data;
}

// If Java serialization is required, use an allow-list filter (Java 9+):
public Object handleTrustedRequest(InputStream input) throws Exception {
    ObjectInputFilter filter = ObjectInputFilter.Config.createFilter(
        "com.myapp.model.*;!*"  // Only allow known classes
    );
    ObjectInputStream ois = new ObjectInputStream(input);
    ois.setObjectInputFilter(filter);
    return ois.readObject();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `object_creation_expression`, `identifier`
- **Detection approach**: Find `method_invocation` nodes calling `readObject` on a variable whose type or initializer is `ObjectInputStream`. Also match `object_creation_expression` constructing `ObjectInputStream` from untrusted sources (method parameters, request.getInputStream(), socket.getInputStream()).
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @target
  name: (identifier) @method
  arguments: (argument_list))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `object_input_stream`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: XMLDecoder on Untrusted XML

### Description
Using `java.beans.XMLDecoder` to decode XML from untrusted sources. `XMLDecoder` can instantiate arbitrary classes, call methods, and set properties based on the XML content, making it a direct code execution vector.

### Bad Code (Anti-pattern)
```java
public Object decodeConfig(InputStream xmlInput) {
    XMLDecoder decoder = new XMLDecoder(xmlInput); // RCE: XML controls object creation
    Object result = decoder.readObject();
    decoder.close();
    return result;
}
```

### Good Code (Fix)
```java
public Config decodeConfig(InputStream xmlInput) throws Exception {
    // Use JAXB with a schema-bound class instead
    JAXBContext context = JAXBContext.newInstance(Config.class);
    Unmarshaller unmarshaller = context.createUnmarshaller();
    return (Config) unmarshaller.unmarshal(xmlInput);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `method_invocation`, `identifier`
- **Detection approach**: Find `object_creation_expression` nodes constructing `XMLDecoder`. Flag any usage since `XMLDecoder` is inherently unsafe for untrusted input. Also detect `readObject()` calls on `XMLDecoder` instances.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  type: (type_identifier) @class_name
  arguments: (argument_list (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `xml_decoder`
- **Severity**: error
- **Confidence**: high
