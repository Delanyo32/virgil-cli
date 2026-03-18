# Insecure Deserialization -- C++

## Overview
Insecure deserialization in C++ occurs when objects are reconstructed from untrusted binary streams without verifying type identity, version compatibility, or field boundaries. Custom serialization code that reads raw bytes into objects via `reinterpret_cast`, `std::istream::read()`, or C-style casts is particularly vulnerable to type confusion and memory corruption.

## Why It's a Security Concern
C++ programs often implement custom binary serialization for performance. When deserializing from untrusted sources, the lack of type verification means an attacker can supply bytes that are interpreted as a different type than intended, corrupting vtable pointers, overwriting object metadata, or triggering undefined behavior. Polymorphic objects are especially dangerous since corrupted vtables enable arbitrary code execution.

## Applicability
- **Relevance**: medium
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: std::istream, Boost.Serialization, cereal, protobuf (when misused), custom binary protocols

---

## Pattern 1: Deserializing Objects From Untrusted Binary Streams Without Type Verification

### Description
Using `std::istream::read()`, `reinterpret_cast`, or `memcpy` to deserialize objects from binary data without verifying a type tag, magic number, or version field first. This allows type confusion attacks where an attacker supplies bytes for a different class, potentially corrupting vtable pointers or object layout.

### Bad Code (Anti-pattern)
```cpp
class Message {
public:
    virtual void process();
    static Message* deserialize(std::istream& stream) {
        Message* msg = new Message();
        stream.read(reinterpret_cast<char*>(msg), sizeof(Message)); // Type confusion + vtable corruption
        return msg;
    }
};

void handleInput(std::istream& untrusted) {
    Message* msg = Message::deserialize(untrusted);
    msg->process(); // Calls through potentially corrupted vtable
}
```

### Good Code (Fix)
```cpp
enum class MessageType : uint32_t {
    TextMessage = 1,
    BinaryMessage = 2,
};

struct MessageHeader {
    uint32_t magic;
    MessageType type;
    uint32_t version;
    uint32_t payload_size;
};

std::unique_ptr<Message> deserialize(std::istream& stream) {
    MessageHeader header;
    stream.read(reinterpret_cast<char*>(&header), sizeof(header));
    if (header.magic != EXPECTED_MAGIC || header.version != CURRENT_VERSION) {
        throw std::runtime_error("invalid message format");
    }
    if (header.payload_size > MAX_PAYLOAD_SIZE) {
        throw std::runtime_error("payload too large");
    }
    switch (header.type) {
        case MessageType::TextMessage:
            return TextMessage::fromStream(stream, header.payload_size);
        case MessageType::BinaryMessage:
            return BinaryMessage::fromStream(stream, header.payload_size);
        default:
            throw std::runtime_error("unknown message type");
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `template_function`, `cast_expression`, `identifier`
- **Detection approach**: Find patterns where `stream.read()` or `istream::read()` writes into a `reinterpret_cast<char*>(object_ptr)` where the target is a class/struct pointer. Flag when there is no preceding validation of a type tag or magic number (no comparison or switch statement before the cast). Also detect `memcpy` into object pointers from stream buffers.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    argument: (identifier) @stream
    field: (field_identifier) @method)
  arguments: (argument_list
    (cast_expression
      type: (type_descriptor) @cast_type
      value: (_) @target)
    (_) @size))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `binary_stream_no_type_check`
- **Severity**: error
- **Confidence**: medium
