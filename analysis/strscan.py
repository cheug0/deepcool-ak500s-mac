import re, sys, glob, os
pat = re.compile(rb'(?i)VID_|PID_|0x[0-9a-f]{4}|usagePage|hid_|HidD|deepcool|serial|report')
for path in sys.argv[1:]:
    data = open(path, 'rb').read()
    print(f"=== {path} ({len(data)} bytes)")
    strs = re.findall(rb'[\x20-\x7e]{5,}', data)
    seen = set(); n = 0
    for s in strs:
        if pat.search(s) and s not in seen and n < 30:
            seen.add(s); n += 1
            print('  ', s.decode()[:110])
