"""Realistic arena-like case generator (701-shaped). Deterministic per seed."""
import json, random, sys, collections
def nbrs(r,c,H,W):
    # offset hex (odd-r); connectivity check only — use 8-neigh superset minus safe: use 4-neigh subset for safety
    for dr,dc in ((0,1),(0,-1),(1,0),(-1,0)):
        rr,cc=r+dr,c+dc
        if 0<=rr<H and 0<=cc<W: yield rr,cc
TAG='real'
def gen(seed,size,agents,days,short,fuel_mult):
    global TAG
    rng=random.Random(seed); H=W=size
    frac=[rng.uniform(.2,.35),rng.uniform(.18,.3),rng.uniform(.2,.3),rng.uniform(.08,.2)]
    tot=sum(frac); frac=[f/tot for f in frac]
    g=[[rng.choices(range(4),frac)[0] for _ in range(W)] for _ in range(H)]
    for _ in range(4):  # majority smoothing -> blobs
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
    # connectivity: largest non-pond 4-connected component
    comp={}
    best=[]
    for r in range(H):
        for c in range(W):
            if g[r][c]==3 or (r,c) in comp: continue
            q=[(r,c)]; comp[(r,c)]=1; i=0
            while i<len(q):
                a,b=q[i]; i+=1
                for x in nbrs(a,b,H,W):
                    if g[x[0]][x[1]]!=3 and x not in comp: comp[x]=1; q.append(x)
            if len(q)>len(best): best=q
    cells=best[:]; rng.shuffle(cells)
    nser={16:13,24:20,32:27}[size]; nspots=nser+rng.randint(1,max(2,size//6))
    picked=cells[:nspots+agents]
    spots=[]
    brands=list(range(nser))+[rng.randrange(nser) for _ in range(nspots-nser)]
    for (r,c),b in zip(picked[:nspots],brands):
        g[r][c]=0; spots.append({"brand":b,"pos":r*W+c,"stocks":rng.randint(1,agents)})
    ag=[]
    for r,c in picked[nspots:]:
        g[r][c]=0; ag.append(r*W+c)
    lo,hi=W+H,(W+H)*4
    if short: hi=int((W+H)*(1.6 if short is True else short))
    ds=[rng.randint(lo,hi) for _ in range(days)]
    return {"case_id":f"fixture:{TAG}-{size}-a{agents}-d{days}-{'short' if short else 'mixed'}-s{seed}","seed":None,"generation_parameters":None,
      "source":{"kind":"fixture","name":f"{TAG}-{size}-a{agents}-d{days}-{'short' if short else 'mixed'}-s{seed}"},
      "startsAt":1778227200,"daySeconds":[60]*days,"daySteps":ds,"map":{"height":H,"width":W,"cells":g},"spots":spots,"agents":ag,
      "fuelLimits":int(ds[0]*fuel_mult),"players":10,"busyThreshold":3,"jammedThreshold":8}
out=sys.argv[1]; mode=sys.argv[2] if len(sys.argv)>2 else 'tune'; names=[]
globals()['TAG']={'tune':'real','holdout':'hold','hard':'hard'}[mode]
seed={'tune':1000,'holdout':5000,'hard':9000}[mode]
for size,agopts in ((16,(4,4,3,5)),(24,(5,5,4,6)),(32,(7,7,6,8))):
    for i in range(8 if mode!='hard' else 4):
        seed+=1; rng=random.Random(seed*7)
        if mode=='hard':
            c=gen(seed,size,agopts[i%4],rng.choice([7,9,10]),short=1.15,fuel_mult=1.0)
        else:
            c=gen(seed,size,agopts[i%4],rng.choice([4,6,7,9,10]),short=(i%2==1),fuel_mult=[1.0,2.4,1.5,3.0][i%4] if i<4 else rng.uniform(1,3))
        p=f"{out}/{c['source']['name']}.json"; json.dump(c,open(p,'w')); names.append(p)
print(','.join(names))
