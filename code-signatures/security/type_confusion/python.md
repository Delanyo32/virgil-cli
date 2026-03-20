# Type Confusion -- Python

## Overview
Type confusion in Python can occur through dynamic type manipulation and monkey-patching, where the runtime type of an object is altered to bypass `isinstance()` checks or other type-based security gates. While Python's duck typing is a feature, it can be exploited when security-critical code relies on `isinstance()` for authorization or input validation and an attacker can influence the type hierarchy at runtime.

## Why It's a Security Concern
Python allows modifying an object's `__class__` attribute at runtime, registering virtual subclasses via `ABCMeta.register()`, and overriding `__instancecheck__` on metaclasses. If security-critical code uses `isinstance()` to gate access -- for example, checking if a request object is an `AdminRequest` -- an attacker who can manipulate the class hierarchy can bypass these checks entirely. This is most dangerous in plugin systems, deserialization pipelines, or applications that process user-supplied class definitions.

## Applicability
- **Relevance**: low
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: abc (ABCMeta), pickle, any framework using isinstance() for authorization gates

---

## Pattern 1: isinstance() Bypass via Monkey-Patching or Dynamic Type Manipulation

### Description
Relying on `isinstance()` for security-critical decisions when the type hierarchy can be influenced by untrusted code. An attacker can reassign an object's `__class__` attribute, use `ABCMeta.register()` to register a class as a virtual subclass, or define a custom `__instancecheck__` metaclass method to make `isinstance()` return `True` for objects that should not pass the check.

### Bad Code (Anti-pattern)
```python
class AdminUser:
    def __init__(self, name):
        self.name = name

class RegularUser:
    def __init__(self, name):
        self.name = name

def delete_database(user):
    # Security gate relies solely on isinstance()
    if not isinstance(user, AdminUser):
        raise PermissionError("Only admins can delete the database")
    perform_delete()

# Attacker bypasses the check by reassigning __class__
attacker = RegularUser("mallory")
attacker.__class__ = AdminUser  # now isinstance(attacker, AdminUser) is True
delete_database(attacker)  # succeeds -- security bypass
```

### Good Code (Fix)
```python
class AdminUser:
    __slots__ = ('name', '_auth_token')

    def __init__(self, name, auth_token):
        self.name = name
        self._auth_token = auth_token

def delete_database(user):
    # Do not rely solely on isinstance() for security
    if not isinstance(user, AdminUser):
        raise PermissionError("Only admins can delete the database")

    # Verify actual credentials, not just type identity
    if not verify_auth_token(user._auth_token):
        raise PermissionError("Invalid authentication token")

    # Additional check: verify type has not been monkey-patched
    if type(user) is not AdminUser:
        raise PermissionError("Type integrity check failed")

    perform_delete()
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `identifier`, `if_statement`, `attribute`, `assignment`
- **Detection approach**: Find `if_statement` conditions containing `isinstance()` calls that guard security-sensitive operations (function bodies containing authorization keywords like `raise PermissionError`, `raise Forbidden`, `deny`, or `abort`). Also flag `assignment` nodes where the left side is an `attribute` with name `__class__` -- this indicates runtime type manipulation that could be used to bypass `isinstance()` checks.
- **S-expression query sketch**:
```scheme
(assignment
  left: (attribute
    attribute: (identifier) @attr)
  right: (_) @new_class
  (#eq? @attr "__class__"))
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `isinstance_bypass`
- **Severity**: warning
- **Confidence**: medium
