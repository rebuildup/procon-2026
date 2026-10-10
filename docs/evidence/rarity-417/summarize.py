import json
from pathlib import Path
root=Path(__file__).parent
all_pairs=[]
summaries=[]
for stage,n,prefix in [('hard',12,'fixture:hard-'),('holdout',24,'fixture:hold-'),('official',9,'')]:
    raw=[json.loads(line) for line in (root/stage/'runs.jsonl').read_text().splitlines()]
    selected=[r for r in raw if r['case_id'].startswith(prefix)]
    assert len(selected)==n*2,(stage,len(selected))
    groups={}
    for r in selected:
        assert r['source_commit_sha']=='d87c66d169087e917881fb4eb676a3b2cac8b06c'
        assert r['scored'] and r['status']=='success'
        assert not any(r[k] for k in ['illegal_action_count','fuel_short_count','rejected_response_count','day_budget_exceeded_days','timeout'])
        rarity=int(next(part.split('=')[1] for part in r['candidate_id'].split(':') if part.startswith('rarity=')))
        group=groups.setdefault(r['case_id'],{})
        assert rarity not in group
        group[rarity]=r
    pairs=[]
    for case,group in sorted(groups.items()):
        assert set(group)=={0,2}
        a,b=group[0],group[2]
        assert a['agent_kinds']==b['agent_kinds']
        sa=[a[k] for k in ['variety','daily_variety_sum','total_balls']]
        sb=[b[k] for k in ['variety','daily_variety_sum','total_balls']]
        oa=sa+[-(a['response_time_us']//1000)]
        ob=sb+[-(b['response_time_us']//1000)]
        outcome=lambda x,y:'win' if x>y else 'loss' if x<y else 'tie'
        pairs.append(dict(stage=stage,case_id=case,baseline=sa,rarity2=sb,delta=[y-x for x,y in zip(sa,sb)],score_outcome=outcome(sb,sa),official_outcome=outcome(ob,oa),baseline_response_us=a['response_time_us'],rarity2_response_us=b['response_time_us'],baseline_max_day_us=max(a['daily_response_us']),rarity2_max_day_us=max(b['daily_response_us'])))
    summary=dict(stage=stage,cases=n,score_counts={x:sum(p['score_outcome']==x for p in pairs) for x in ['win','tie','loss']},official_counts={x:sum(p['official_outcome']==x for p in pairs) for x in ['win','tie','loss']},delta_totals=[sum(p['delta'][i] for p in pairs) for i in range(3)],v_regressions=sum(p['delta'][0]<0 for p in pairs),d_regressions=sum(p['delta'][1]<0 for p in pairs),max_day_us=max(max(p['baseline_max_day_us'],p['rarity2_max_day_us']) for p in pairs))
    summaries.append(summary)
    all_pairs.extend(pairs)
assert len({p['case_id'] for p in all_pairs})==45
output=dict(summaries=summaries,pairs=all_pairs)
(root/'paired-summary.json').write_text(json.dumps(output,indent=2)+'\n')
print(json.dumps(summaries,indent=2))
