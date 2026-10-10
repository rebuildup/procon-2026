import json,sys,collections
g=collections.defaultdict(dict)
for l in open(sys.argv[1]):
    r=json.loads(l)
    if 'openapi' in r['case_id']: continue
    c=r['candidate_id']; c='fleet' if c.startswith('fleet') else c.replace('convoy:budget=official:seed=42','convoy').replace(':chains=4','')
    g[c][r['case_id'].split(':')[-1]]=((r['variety'],r['daily_variety_sum'],r['total_balls']), r['status'], r['illegal_action_count']+r['fuel_short_count']+r['rejected_response_count'], max(r['daily_response_us'] or [0])/1e6)
base=g['fleet']; cases=sorted(base)
print('board'.ljust(22), *[c[-14:].rjust(15) for c in g])
for k in cases: print(k[-20:].ljust(22), *[str(g[c][k][0][1:]).rjust(15) if k in g[c] else '-'.rjust(15) for c in g])
for c,v in g.items():
    ks=[k for k in cases if k in v]
    print(c.ljust(34),'W',sum(v[k][0]>base[k][0] for k in ks),'L',sum(v[k][0]<base[k][0] for k in ks),'Dlow',sum(v[k][0][:2]<base[k][0][:2] for k in ks),'V',sum(v[k][0][0] for k in ks),'D',sum(v[k][0][1] for k in ks),'B',sum(v[k][0][2] for k in ks),'bad',sum(v[k][2]+(v[k][1]!='success') for k in ks),'max_s',round(max(v[k][3] for k in ks),1))
