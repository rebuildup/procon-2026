import json,sys
rows=[json.loads(l) for l in open(sys.argv[1])]
print(sorted(rows[0].keys()))
b={};o={}
for r in rows:
    if r['case_id']=='fixture:openapi-sample': continue
    (b if r['candidate_id'].startswith('fleet') else o)[r['case_id']]=r
w=d=l=0;T=[0]*6;bad=0;mx=0
for k,r in sorted(o.items()):
    x=b[k]; a=(r['variety'],r['daily_variety_sum'],r['total_balls']); c=(x['variety'],x['daily_variety_sum'],x['total_balls'])
    w+=a>c; d+=a==c; l+=a<c
    for i in range(3): T[i]+=a[i]; T[3+i]+=c[i]
    bad+=r['illegal_action_count']+r['fuel_short_count']+r['rejected_response_count']+(r['status']!='success')
    t=[v for kk,v in r.items() if 'ms' in kk and isinstance(v,(int,float))]
    tag='WIN ' if a>c else ('LOSS' if a<c else 'same')
    print(f"{tag} {k.split(':')[-1]:34} convoy {a} fleet {c} status {r['status']} times { {kk:v for kk,v in r.items() if 'ms' in kk and not isinstance(v,list)} }")
print(f"W{w} D{d} L{l} | V {T[0]} vs {T[3]} | D {T[1]} vs {T[4]} | B {T[2]} vs {T[5]} | bad {bad}")
