import json,re,sys
d=sys.argv[1]
log=re.sub(r'\x1b\[[0-9;]*m','',open(d+'/cli.log').read())
pred={}
for m in re.finditer(r'(\d\d:\d\d:\d\d\.\d+)Z\s+INFO fleet day plan day=(\d+) clean=(\w+) chains=\d+ model_series=(\d+) model_balls=(\d+) replay_series=(\d+) replay_balls=(\d+) tomorrow_series=\d+ tomorrow_balls=\d+ predicted_ends=\[(.*?)\]\n',log):
    if 'agent kinds selected' in log and m.start()<log.index('agent kinds selected'): continue
    pred[int(m.group(2))]=([tuple(map(int,x.split(','))) for x in re.findall(r'\((\d+, \d+)\)',m.group(8).replace(', ',','))], m.group(3), int(m.group(5)))
ok=bad=0
for e in json.load(open(d+'/requests.json')):
    r=e.get('response')
    if e['method']=='GET' and e['path']=='/' and isinstance(r,dict) and 'day' in r and r['day']>0 and r['day']-1 in pred:
        got=[(a['pos'],a['fuel']) for a in r['agents']]; exp=pred[r['day']-1][0]; kinds=[a['kind'] for a in r['agents']]
        same=all(g[0]==x[0] and (k==1 or g[1]==x[1]) for g,x,k in zip(got,exp,kinds)); ok+=same; bad+=not same
        if not same: print('MISMATCH day',r['day'],got,exp)
print('days',len(pred),'server-state matches',ok,'mismatches',bad,'clean',set(v[1] for v in pred.values()),'balls/day',[pred[k][2] for k in sorted(pred)],'sum',sum(v[2] for v in pred.values()))
