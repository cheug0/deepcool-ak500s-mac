import re
data = open(r'E:\test_exe\DeepCool\resources\service\x64\deep_service.exe','rb').read()
strs = re.findall(rb'[\x20-\x7e]{4,}', data)
# also UTF-16LE strings
u16 = re.findall(rb'(?:[\x20-\x7e]\x00){4,}', data)
print("### ASCII strings of interest ###")
pat = re.compile(rb'(?i)deepcool|cooler|temp|fan|pump|display|ak\d|ch\d|lt\d|ld|lp\d|lq')
seen = set()
for s in strs:
    if pat.search(s) and s not in seen and len(s) > 4:
        seen.add(s)
print('\n'.join(sorted(x.decode() for x in seen))[:3000])
print("\n### UTF-16LE strings ###")
seen2 = set()
for s in u16:
    t = s.decode('utf-16-le')
    if pat.search(t.encode('latin1','ignore')) and t not in seen2:
        seen2.add(t)
print('\n'.join(sorted(seen2))[:1500])
