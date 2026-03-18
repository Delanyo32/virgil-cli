# Insecure Deserialization -- Python

## Overview
Insecure deserialization in Python commonly involves `pickle` and `yaml` modules, both of which can execute arbitrary code during deserialization. These are among the most exploited deserialization vectors in the Python ecosystem.

## Why It's a Security Concern
Python's `pickle` module is explicitly documented as unsafe for untrusted data — it can execute arbitrary Python code during unpickling via the `__reduce__` protocol. Similarly, `yaml.load()` with the default loader can instantiate arbitrary Python objects. Both represent well-known remote code execution vectors.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: pickle, cPickle, shelve, PyYAML, ruamel.yaml, Django, Flask, Celery

---

## Pattern 1: pickle.loads() on Untrusted Data

### Description
Calling `pickle.loads()`, `pickle.load()`, `cPickle.loads()`, or `shelve.open()` on data received from untrusted sources (network, user uploads, external APIs). The `__reduce__` method on pickled objects allows arbitrary code execution during deserialization.

### Bad Code (Anti-pattern)
```python
import pickle

def handle_request(request):
    data = pickle.loads(request.body)  # RCE: attacker crafts malicious pickle
    process(data)
```

### Good Code (Fix)
```python
import json
from pydantic import BaseModel

class RequestData(BaseModel):
    name: str
    value: int

def handle_request(request):
    raw = json.loads(request.body)
    data = RequestData(**raw)  # Schema-validated, no code execution
    process(data)
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `identifier`
- **Detection approach**: Find `call` nodes where the function is an `attribute` with object `pickle` (or `cPickle`) and attribute `loads` or `load`. Also match bare `loads`/`load` if `pickle` was imported with `from pickle import loads`.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
  arguments: (argument_list (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `pickle_loads`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: yaml.load() Without yaml.safe_load()

### Description
Calling `yaml.load()` without specifying `Loader=yaml.SafeLoader` (or using `yaml.safe_load()` instead). The default loader in PyYAML can instantiate arbitrary Python objects via YAML tags like `!!python/object/apply:os.system`.

### Bad Code (Anti-pattern)
```python
import yaml

def load_config(config_string):
    return yaml.load(config_string)  # Unsafe: default Loader allows code execution
```

### Good Code (Fix)
```python
import yaml

def load_config(config_string):
    return yaml.safe_load(config_string)  # Safe: only allows basic YAML types
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `identifier`, `keyword_argument`
- **Detection approach**: Find `call` nodes where the function is `yaml.load`. Check the argument list for absence of `Loader=yaml.SafeLoader` or `Loader=yaml.CSafeLoader` keyword argument. Flag when no safe loader is specified.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
  arguments: (argument_list (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `yaml_unsafe_load`
- **Severity**: error
- **Confidence**: high
