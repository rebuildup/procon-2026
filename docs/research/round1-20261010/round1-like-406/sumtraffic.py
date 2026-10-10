import json,sys,collections
g=collections.defaultdict(dict)
for l in open(sys.argv[1]):
    r=json.loads(l); c='fleet' if r['options'][0].startswith('fleet') else 'convoy'
    g[c][r['label']]=((r['V'],r['D'],r['B']), r['illegal_action_count']+r['fuel_short_count']+r['rejected_response_count'], max(r['decide_ms'])/1000)
b=g['fleet']; v=g['convoy']; ks=sorted(set(b)&set(v))
print('n',len(ks),'W',sum(v[k][0]>b[k][0] for k in ks),'same',sum(v[k][0]==b[k][0] for k in ks),'L',sum(v[k][0]<b[k][0] for k in ks),'D lower',sum(v[k][0][:2]<b[k][0][:2] for k in ks),'| D',sum(v[k][0][1] for k in ks),'vs',sum(b[k][0][1] for k in ks),'| B',sum(v[k][0][2] for k in ks),'vs',sum(b[k][0][2] for k in ks),'| bad',sum(v[k][1]+b[k][1] for k in ks),'| convoy max s',max(v[k][2] for k in ks))
print('losses',{k:(v[k][0],b[k][0]) for k in ks if v[k][0]<b[k][0]})
