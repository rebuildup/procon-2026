import json, random, sys, collections
def gen(seed):
    rng=random.Random(seed); W=rng.randint(8,32); H=rng.randint(8,32)
    frac=[rng.uniform(.1,.5),rng.uniform(.0,.4),rng.uniform(.0,.4),rng.uniform(.0,.35)]
    tot=sum(frac); frac=[f/tot for f in frac]
    g=[[rng.choices(range(4),frac)[0] for _ in range(W)] for _ in range(H)]
    for _ in range(rng.randint(0,4)):
        n=[[0]*W for _ in range(H)]
        for r in range(H):
            for c in range(W):
                cnt=collections.Counter()
                for dr in (-1,0,1):
                    for dc in (-1,0,1):
                        rr,cc=r+dr,c+dc
                        if 0<=rr<H and 0<=cc<W: cnt[g[rr][cc]]+=1+(dr==0 and dc==0)*rng.random()*2
                n[r][c]=max(cnt,key=lambda k:(cnt[k],rng.random()))
        g=n
    agents=rng.randint(3,8)
    cells=[(r,c) for r in range(H) for c in range(W)]; rng.shuffle(cells)  # NOT connectivity-filtered: unreachable spots allowed
    nspots=rng.randint(agents,min(32,len(cells)//4)); nser=rng.randint(1,nspots)
    spots=[]
    for i,(r,c) in enumerate(cells[:nspots]):
        g[r][c]=0; spots.append({"brand":i if i<nser else rng.randrange(nser),"pos":r*W+c,"stocks":rng.randint(1,agents)})
    ag=[]
    for i in range(agents):
        m=rng.random()
        if m<0.2 and ag: ag.append(ag[-1])                      # stacked agents
        elif False: pass
        else:
            r,c=cells[nspots+i]; g[r][c]=0; ag.append(r*W+c)
    days=rng.randint(1,10); lo,hi=W+H,(W+H)*4
    ds=[rng.randint(lo,rng.choice([lo,hi,int(lo*1.5)])) for _ in range(days)]
    name=f"fuzz-{seed}"
    return {"case_id":f"fixture:{name}","seed":None,"generation_parameters":None,"source":{"kind":"fixture","name":name},
      "startsAt":1778227200,"daySeconds":[60]*days,"daySteps":ds,"map":{"height":H,"width":W,"cells":g},"spots":spots,"agents":ag,
      "fuelLimits":int(ds[0]*rng.uniform(1,3)),"players":rng.randint(2,10),"busyThreshold":rng.randint(1,5),"jammedThreshold":rng.randint(6,10)}
out=sys.argv[1]; names=[]
for seed in range(int(sys.argv[2]),int(sys.argv[3])):
    c=gen(seed); p=f"{out}/{c['source']['name']}.json"; json.dump(c,open(p,'w')); names.append(p)
print(','.join(names))
