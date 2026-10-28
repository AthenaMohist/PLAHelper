import json,sys
d=json.load(open(sys.argv[1],encoding='utf-8'))
nodes=d['nodes']; strings=d['strings']; E=d['edges']
k=6; w=3
n=len(nodes)//k
from collections import defaultdict
src=defaultdict(set)
eoff=0
for i in range(n):
    off=i*k
    ecnt=nodes[off+4]
    for j in range(ecnt):
        eo=eoff+j*w
        nameidx=E[eo+1]
        nm=strings[nameidx] if isinstance(nameidx,int) and nameidx<len(strings) else None
        if nm in ('selection','shading','viewports'):
            src[i].add(nm)
    eoff+=ecnt*w
for i,s in src.items():
    if len(s)>=3:
        off=i*k
        print(f'FOUND:{nodes[off+2]}')
        break
else:
    print('NOT_FOUND')
