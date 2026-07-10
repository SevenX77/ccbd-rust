#!/usr/bin/env python3
import os
import re
import sys

def strip_test_blocks(text):
    result = []
    i = 0
    n = len(text)
    while i < n:
        # Check if we are starting a test module or unit test block
        if text[i:].startswith('mod tests') or text[i:].startswith('#[cfg(test)]'):
            # Find the opening brace '{'
            brace_idx = text.find('{', i)
            if brace_idx != -1:
                # Track braces to find the matching closing brace
                brace_count = 1
                j = brace_idx + 1
                while j < n and brace_count > 0:
                    if text[j] == '{':
                        brace_count += 1
                    elif text[j] == '}':
                        brace_count -= 1
                    j += 1
                i = j
                continue
        result.append(text[i])
        i += 1
    return "".join(result)

raw_update_rx = re.compile(r'UPDATE\s+jobs\s+SET\s+status', re.IGNORECASE)

violations = []
for root, dirs, files in os.walk('src'):
    for file in files:
        if not file.endswith('.rs'):
            continue
        path = os.path.join(root, file)
        
        # Exclude only the gate itself (state_machine.rs is checked)
        if path == 'src/db/job_state.rs':
            continue
            
        with open(path, 'r', encoding='utf-8') as f:
            content = f.read()
            
        # Strip test blocks
        prod_content = strip_test_blocks(content)
        
        # Merge backslash line continuations in Rust string literals
        prod_content = re.sub(r'\\\s*\n\s*', ' ', prod_content)
        
        if raw_update_rx.search(prod_content):
            violations.append(path)

if violations:
    print("Error: Found raw UPDATE jobs SET status updates in non-test production code:")
    for path in violations:
        print(f"  {path}")
    sys.exit(1)
else:
    print("Success: No raw UPDATE jobs SET status updates found outside of tests and gate implementations.")
    sys.exit(0)
