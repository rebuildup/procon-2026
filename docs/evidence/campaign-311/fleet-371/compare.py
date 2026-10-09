import json,sys
# usage: cmp2.py base.jsonl base_cand other.jsonl other_cand [label]
def load(f,cand):
    d={}
    for l in open(f):
        r=json.loads(l)
        if r['candidate_id'].startswith(cand) and r['case_id']!='fixture:openapi-sample': d[r['case_id']]=r
    return d
b=load(sys.argv[1],sys.argv[2]); o=load(sys.argv[3],sys.argv[4])
w=dr=l=0; T=[0]*6; bad=0; vl=dl=0; loss=[]
for k,r in o.items():
    x=b[k]; a=(r['variety'],r['daily_variety_sum'],r['total_balls']); c=(x['variety'],x['daily_variety_sum'],x['total_balls'])
    w+=a>c; dr+=a==c; l+=a<c; vl+=a[0]<c[0]; dl+=(a[0]==c[0] and a[1]<c[1])
    if a<c: loss.append((k.split(':')[1],a,c))
    for i in range(3): T[i]+=a[i]; T[3+i]+=c[i]
    bad+=r['illegal_action_count']+r['fuel_short_count']+r['rejected_response_count']+(r['status']!='success')
print(f"{(sys.argv[5] if len(sys.argv)>5 else sys.argv[3].split('/')[-2]):26} W{w} D{dr} L{l} (V-loss {vl}, D-loss {dl}) V {T[0]} vs {T[3]} | D {T[1]} vs {T[4]} | B {T[2]} vs {T[5]} bad={bad}")
if '-v' in sys.argv:
    for x in loss: print('    LOSS',x)
