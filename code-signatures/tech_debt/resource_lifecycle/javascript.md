# Resource Lifecycle -- JavaScript

## Overview
Resources that are acquired but never properly released cause memory leaks, file descriptor exhaustion, and degraded application performance over time. In JavaScript, the most common manifestations are event listeners that are never removed and timers (setInterval/setTimeout) that are never cleared.

## Why It's a Tech Debt Concern
Event listeners that are added without corresponding removal accumulate over time, especially in single-page applications and long-lived Node.js processes. Each leaked listener holds a reference to its closure and all captured variables, preventing garbage collection and steadily increasing memory consumption. Timers created with setInterval or setTimeout without stored references cannot be cleared, leading to callbacks that continue executing after their owning component or context has been destroyed, causing errors, wasted CPU cycles, and potential data corruption.

## Applicability
- **Relevance**: high (SPAs, React components, Node.js servers)
- **Languages covered**: `.js`, `.jsx`
- **Frameworks/libraries**: React (component mount/unmount), Node.js EventEmitter, DOM APIs

---

## Pattern 1: Event Listener Not Removed on Cleanup

### Description
Adding an event listener with `addEventListener` or `.on()` without a corresponding `removeEventListener` or `.off()` call in a cleanup path. In React, this means adding a listener in `useEffect` or `componentDidMount` without removing it in the cleanup function or `componentWillUnmount`. In Node.js, attaching listeners to EventEmitters without removing them leads to the "MaxListenersExceededWarning".

### Bad Code (Anti-pattern)
```javascript
// React -- listener added but never removed
useEffect(() => {
  window.addEventListener('resize', handleResize);
  document.addEventListener('keydown', handleKeyDown);
  // No cleanup function returned
}, []);

// Node.js -- listener accumulates on every call
function watchFile(emitter, callback) {
  emitter.on('change', callback);
  // No removal -- called repeatedly, listeners pile up
}

// Vanilla JS -- listener on class instance, never removed
class JsonEditor {
  init() {
    this.textarea.addEventListener('input', this.onInput.bind(this));
    this.saveBtn.addEventListener('click', this.onSave.bind(this));
    // .bind() creates new function references, making removal impossible
  }
}
```

### Good Code (Fix)
```javascript
// React -- cleanup function removes listeners
useEffect(() => {
  window.addEventListener('resize', handleResize);
  document.addEventListener('keydown', handleKeyDown);
  return () => {
    window.removeEventListener('resize', handleResize);
    document.removeEventListener('keydown', handleKeyDown);
  };
}, []);

// Node.js -- track and remove listener
function watchFile(emitter, callback) {
  emitter.on('change', callback);
  return () => emitter.off('change', callback);
}

// Vanilla JS -- store bound references for removal
class JsonEditor {
  init() {
    this._onInput = this.onInput.bind(this);
    this._onSave = this.onSave.bind(this);
    this.textarea.addEventListener('input', this._onInput);
    this.saveBtn.addEventListener('click', this._onSave);
  }
  destroy() {
    this.textarea.removeEventListener('input', this._onInput);
    this.saveBtn.removeEventListener('click', this._onSave);
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `property_identifier`
- **Detection approach**: Find `call_expression` nodes where the function is a `member_expression` with `property_identifier` equal to `addEventListener` or `on`. Then check if the enclosing function body or class body contains a corresponding `removeEventListener` or `off` call. In React `useEffect` callbacks, check if the arrow function body returns a cleanup function containing the removal call.
- **S-expression query sketch**:
  ```scheme
  (call_expression
    function: (member_expression
      property: (property_identifier) @method_name)
    arguments: (arguments
      (string) @event_name
      (_) @handler))
  ```

### Pipeline Mapping
- **Pipeline name**: `event_listener_leak`
- **Pattern name**: `listener_not_removed`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: setInterval/setTimeout Without Cleanup Reference

### Description
Calling `setInterval` or `setTimeout` without storing the returned timer ID in a variable, making it impossible to clear the timer later. This is especially problematic with `setInterval`, where the callback continues executing indefinitely. In components or request handlers, leaked intervals continue running after the owning context is gone.

### Bad Code (Anti-pattern)
```javascript
// setInterval with no stored reference -- cannot be cleared
function startPolling(url) {
  setInterval(async () => {
    const data = await fetch(url).then(r => r.json());
    updateDisplay(data);
  }, 5000);
}

// setTimeout in a loop with no cleanup tracking
function scheduleRetries(task, maxRetries) {
  for (let i = 0; i < maxRetries; i++) {
    setTimeout(() => task(), i * 1000);
    // Cannot cancel these if the operation succeeds early
  }
}

// React component -- interval not cleared on unmount
useEffect(() => {
  setInterval(() => {
    setCount(c => c + 1);
  }, 1000);
}, []);
```

### Good Code (Fix)
```javascript
// Store interval ID and clear on cleanup
function startPolling(url) {
  const intervalId = setInterval(async () => {
    const data = await fetch(url).then(r => r.json());
    updateDisplay(data);
  }, 5000);
  return () => clearInterval(intervalId);
}

// Track timeout IDs for cancellation
function scheduleRetries(task, maxRetries) {
  const timers = [];
  for (let i = 0; i < maxRetries; i++) {
    timers.push(setTimeout(() => task(), i * 1000));
  }
  return () => timers.forEach(id => clearTimeout(id));
}

// React component -- clear interval on unmount
useEffect(() => {
  const intervalId = setInterval(() => {
    setCount(c => c + 1);
  }, 1000);
  return () => clearInterval(intervalId);
}, []);
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `expression_statement`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the function is the identifier `setInterval` or `setTimeout`. Check if the call is inside an `expression_statement` (meaning its return value is discarded) rather than being assigned to a variable via `variable_declarator` or `assignment_expression`. Discard cases are timer leaks.
- **S-expression query sketch**:
  ```scheme
  ;; Timer call whose return value is discarded
  (expression_statement
    (call_expression
      function: (identifier) @timer_fn))
  ```

### Pipeline Mapping
- **Pipeline name**: `event_listener_leak`
- **Pattern name**: `timer_without_cleanup`
- **Severity**: warning
- **Confidence**: high
