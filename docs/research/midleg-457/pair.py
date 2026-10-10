import json,collections,sys,glob,os
# pair.py RESULTS.jsonl KEY  -> paired comparison of cands with/without KEY substring, grouped by board family
by=collections.defaultdict(dict)
for l in open(sys.argv[1]):
    r=json.loads(l)
    if 'error' in r: print(str(r)[:200]); continue
    sd=r['cand'].split('seed=')[1].split(':')[0]
    by[(r['case'],sd)]['new' if sys.argv[2] in r['cand'] else 'base']=(r['V'],r['D'],r['B'],r['bad'],r['np'])
T=collections.defaultdict(lambda:collections.defaultdict(list))
for (c,sd),d in by.items():
    if len(d)<2: continue
    fam=('arena' if c.startswith('arena') else c[:10])+'-np'+str(d['base'][4])
    for f in (fam,'ALL'): T[f]['x'].append((d['base'][:3],d['new'][:3],d['new'][3]))
for f,t in sorted(T.items()):
    x=t['x']; n=len(x); w=sum(b>a for a,b,_ in x); l=sum(b<a for a,b,_ in x)
    dV=sum(b[0]-a[0] for a,b,_ in x); dD=sum(b[1]-a[1] for a,b,_ in x); dw=sum(b[1]<a[1] for a,b,_ in x); diffs=[b[2]-a[2] for a,b,_ in x]; B0=sum(a[2] for a,b,_ in x)
    m=sum(diffs)/n; sd=(sum((v-m)**2 for v in diffs)/max(n-1,1))**.5
    print(f"{f:18s} n={n} W/T/L {w}/{n-w-l}/{l} dV {dV:+d} dD {dD:+d} (D worse {dw}) dB {sum(diffs):+d} ({100*sum(diffs)/B0:+.1f}%) per-pair {m:+.1f} ± {1.96*sd/n**.5:.1f} worst {min(diffs)} bad {sum(b for _,_,b in x)}")
