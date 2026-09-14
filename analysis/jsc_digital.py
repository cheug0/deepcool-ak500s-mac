import re
data = open(r'E:\ak500s_mac\analysis\asar_extracted\out\main\index.jsc','rb').read()
names = set(m.group().decode() for m in re.finditer(rb'[A-Z][A-Z0-9-]{2,20}-DIGITAL', data))
print("product names in bytecode:", sorted(names))
# check context of "device pid is"
for m in re.finditer(rb'device pid is', data):
    s = max(0, m.start()-400)
    chunk = data[s:m.end()+400]
    txt = re.sub(rb'[^\x20-\x7e]', b'.', chunk).decode()
    print('CTX:', txt[:600])
    break
# find "getDeviceByPidAndVid" context
for m in re.finditer(rb'getDeviceByPidAndVid', data):
    s = max(0, m.start()-600)
    chunk = data[s:m.end()+200]
    txt = re.sub(rb'[^\x20-\x7e]', b'.', chunk).decode()
    print('CTX2:', txt[:800])
    break
