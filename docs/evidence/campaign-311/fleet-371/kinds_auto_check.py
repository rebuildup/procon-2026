import json,sys,glob,subprocess,os
from concurrent.futures import ThreadPoolExecutor
S=os.path.dirname(os.path.abspath(sys.argv[0])); BIN=sys.argv[1]; sel=sys.argv[2]; full=sys.argv[3]
files=sorted(glob.glob(S+'/corpus/*.json')+glob.glob(S+'/corpus-holdout/*.json')+glob.glob(S+'/corpus-hard/*.json'))+['crates/bench/tests/fixtures/arena-701.json']
def run(j):
    f,body,p,cand=j
    out=subprocess.run([BIN,'--worker','--candidate-id',cand,'--kinds',p,'--case-json',body],capture_output=True,text=True).stdout
    r=json.loads(out); return (r['variety'],r['daily_variety_sum'],r['total_balls'])
jobs=[];meta=[]
for f in files:
    body=open(f).read(); n=len(json.loads(body)['agents'])
    pats=[','.join('S' if j==0 else 'P' for j in range(n))]+[','.join(['P']*n)]
    for p in pats: jobs.append((f,body,p,sel)); meta.append((f,p))
with ThreadPoolExecutor(8) as ex: res=list(ex.map(run,jobs))
pick={}
MARGIN=float(sys.argv[4]) if len(sys.argv)>4 else 1.08
one={};zero={}
for (f,p),sc in zip(meta,res):
    if 'S' in p:
        if f not in one or sc>one[f][0]: one[f]=(sc,p)
    else: zero[f]=(sc,p)
for f in one:
    o=one[f]; z=zero[f]
    pick[f]=z if (z[0][:2]>=o[0][:2] and z[0][2]>=o[0][2]*MARGIN) else o
jobs2=[];meta2=[]
for f in files:
    body=open(f).read(); n=len(json.loads(body)['agents'])
    first=','.join('S' if j==0 else 'P' for j in range(n))
    jobs2.append((f,body,first,full)); jobs2.append((f,body,pick[f][1],full))
with ThreadPoolExecutor(8) as ex: res2=list(ex.map(run,jobs2))
w=d=l=0;tf=[0,0,0];ta=[0,0,0];zero=0
for i,f in enumerate(files):
    a,b=res2[2*i],res2[2*i+1]
    w+=b>a;d+=b==a;l+=b<a
    for k in range(3): tf[k]+=a[k];ta[k]+=b[k]
    z='S' not in pick[f][1]; zero+=z
    if b!=a and (z or b[:2]!=a[:2]): print(os.path.basename(f)[:-5],'first',a,'auto',b,pick[f][1])
print(f"auto vs S-first: W{w} D{d} L{l}  first {tf} auto {ta}  zero-supply picks {zero}")
