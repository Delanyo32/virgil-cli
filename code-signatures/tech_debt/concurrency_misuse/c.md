# Concurrency Misuse -- C

## Overview
C provides low-level concurrency through POSIX threads (pthreads), where thread safety is entirely the programmer's responsibility. Data races from unsynchronized access to shared variables and deadlocks from mutexes not released on error paths are the two most critical concurrency defects in C programs.

## Why It's a Tech Debt Concern
Data races in C are undefined behavior per the C11 standard — the compiler is free to optimize code in ways that produce completely unpredictable results. Unlike higher-level languages, there are no runtime checks or exceptions: corrupted data silently propagates. Mutex leaks on error paths cause deadlocks that only manifest under specific failure conditions, making them extremely difficult to reproduce in testing but devastating in production.

## Applicability
- **Relevance**: high
- **Languages covered**: `.c`, `.h`
- **Frameworks/libraries**: pthreads, C11 threads, OpenMP

---

## Pattern 1: Data Race -- Shared Variable Without Mutex

### Description
Multiple threads read and write to the same global or shared variable without protecting access with a `pthread_mutex_t` or C11 `_Atomic` qualifier. The read-modify-write sequence (`counter++`) is not atomic on most architectures, leading to lost updates, torn reads, and undefined behavior.

### Bad Code (Anti-pattern)
```c
#include <pthread.h>
#include <stdio.h>

int shared_counter = 0;
int shared_buffer[1024];
int buffer_index = 0;

void *worker(void *arg) {
    for (int i = 0; i < 100000; i++) {
        shared_counter++;  // Data race: read-modify-write is not atomic

        shared_buffer[buffer_index] = i;  // Race on buffer_index
        buffer_index++;                    // Race: may overflow or skip slots
    }
    return NULL;
}

int main(void) {
    pthread_t threads[4];
    for (int i = 0; i < 4; i++) {
        pthread_create(&threads[i], NULL, worker, NULL);
    }
    for (int i = 0; i < 4; i++) {
        pthread_join(threads[i], NULL);
    }
    printf("Counter: %d\n", shared_counter);  // Will not be 400000
    return 0;
}
```

### Good Code (Fix)
```c
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>

atomic_int shared_counter = 0;

pthread_mutex_t buffer_mutex = PTHREAD_MUTEX_INITIALIZER;
int shared_buffer[1024];
int buffer_index = 0;

void *worker(void *arg) {
    for (int i = 0; i < 100000; i++) {
        atomic_fetch_add(&shared_counter, 1);  // Atomic increment

        pthread_mutex_lock(&buffer_mutex);
        if (buffer_index < 1024) {
            shared_buffer[buffer_index] = i;
            buffer_index++;
        }
        pthread_mutex_unlock(&buffer_mutex);
    }
    return NULL;
}

int main(void) {
    pthread_t threads[4];
    for (int i = 0; i < 4; i++) {
        pthread_create(&threads[i], NULL, worker, NULL);
    }
    for (int i = 0; i < 4; i++) {
        pthread_join(threads[i], NULL);
    }
    printf("Counter: %d\n", atomic_load(&shared_counter));
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `declaration` (global), `update_expression`, `assignment_expression`, `call_expression`
- **Detection approach**: Find functions that are passed to `pthread_create` as the start routine (third argument). Within those functions, find `update_expression` (e.g., `counter++`) or `assignment_expression` targeting identifiers that are declared at file scope (global variables). Flag when no `pthread_mutex_lock` call precedes the access within the same block. Also check for `_Atomic` or `atomic_` type qualifiers on the variable declaration.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (_)
    (_)
    (identifier) @thread_func
    (_)))

(update_expression
  argument: (identifier) @modified_var)
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `data_race_no_mutex`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Mutex Not Released on Error Path

### Description
A `pthread_mutex_lock()` is called but `pthread_mutex_unlock()` is not reached on all code paths — particularly early returns, `goto` error labels, or branches that skip the unlock. This causes a permanent deadlock the next time any thread tries to acquire the same mutex.

### Bad Code (Anti-pattern)
```c
pthread_mutex_t db_mutex = PTHREAD_MUTEX_INITIALIZER;

int update_record(int id, const char *value) {
    pthread_mutex_lock(&db_mutex);

    char *query = build_query(id, value);
    if (query == NULL) {
        return -1;  // ERROR: mutex not unlocked
    }

    int result = execute_query(query);
    free(query);

    if (result < 0) {
        log_error("Query failed for id %d", id);
        return -1;  // ERROR: mutex not unlocked
    }

    pthread_mutex_unlock(&db_mutex);
    return 0;
}
```

### Good Code (Fix)
```c
pthread_mutex_t db_mutex = PTHREAD_MUTEX_INITIALIZER;

int update_record(int id, const char *value) {
    int ret = -1;

    pthread_mutex_lock(&db_mutex);

    char *query = build_query(id, value);
    if (query == NULL) {
        goto cleanup;
    }

    int result = execute_query(query);
    free(query);

    if (result < 0) {
        log_error("Query failed for id %d", id);
        goto cleanup;
    }

    ret = 0;

cleanup:
    pthread_mutex_unlock(&db_mutex);
    return ret;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `return_statement`, `function_definition`, `if_statement`
- **Detection approach**: Find functions containing `pthread_mutex_lock()` calls. Track all `return_statement` nodes between the lock call and the corresponding `pthread_mutex_unlock()` call. Flag any `return_statement` that is reachable after the lock but before the unlock — these are paths where the mutex is held but never released.
- **S-expression query sketch**:
```scheme
(function_definition
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (identifier) @lock_call))
    (if_statement
      consequence: (compound_statement
        (return_statement) @early_return))))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `mutex_not_released_on_error`
- **Severity**: warning
- **Confidence**: high
