import json,sys,collections,itertools
# slotcv.py TIE_NUM/DEN trainfiles -- testfiles
args=sys.argv[1:]; tie=eval(args[0]); k=args.index('--'); train=args[1:k]; test=args[k+1:]
def load(files):
    by=collections.defaultdict(lambda: collections.defaultdict(list))
    for f in files:
        for l in open(f):
            r=json.loads(l)
            if 'error' in r: continue
            by[r['case']][r['slot']].append((r['V'],r['D'],r['B']))
    return by
tr,te=load(train),load(test)
def pick(slots):
    n=min(len(v) for v in slots.values())
    tot={s:tuple(sum(x[i] for x in v[:n]) for i in range(3)) for s,v in slots.items()}
    bv=max(t[0] for t in tot.values()); bd=max(t[1] for t in tot.values() if t[0]==bv)
    cand=[s for s in sorted(tot) if tot[s][0]==bv and tot[s][1]+n*tie>=bd]
    return max(cand,key=lambda s:(tot[s][2],-s))
agg=collections.Counter(); w=t=l=0
for case in sorted(tr):
    if case not in te: continue
    nslots=len(json.load(open([f for f in [f'cases/off16real/{case}.json',f'cases/off/{case}.json',f'arena/off/{case}.json'] if __import__('os').path.exists(f)][0]))['agents'])
    if len(tr[case])<nslots or len(te[case])<nslots: continue
    s=pick(tr[case])
    def mean(slot): v=te[case][slot]; return tuple(sum(x[i] for x in v)/len(v) for i in range(3))
    a,b=mean(0),mean(s); orc=max(mean(x) for x in te[case])
    for i,nm in enumerate('VDB'): agg['slot0_'+nm]+=a[i]; agg['pick_'+nm]+=b[i]; agg['best_'+nm]+=orc[i]
    w+=b>a; l+=b<a; t+=b==a; agg['n']+=1
    if '-v' in sys.argv: print(case,'pick',s,a,'->',b)
print('tie',tie,'n',agg['n'],'W/T/L',w,t,l,' slot0 V/D/B %.1f %.1f %.1f'%(agg['slot0_V'],agg['slot0_D'],agg['slot0_B']),' pick %.1f %.1f %.1f'%(agg['pick_V'],agg['pick_D'],agg['pick_B']),' testbest %.1f %.1f %.1f'%(agg['best_V'],agg['best_D'],agg['best_B']))
