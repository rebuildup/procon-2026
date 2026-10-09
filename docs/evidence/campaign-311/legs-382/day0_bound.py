"""Exact first-day variety with fuel: per patrol, Pareto (time, fuel) fronts over
(last spot, series mask); legs use the full time/fuel Pareto set of paths. No refuel."""
import json,sys,heapq
def run(f,day=0):
    d=json.load(open(f)); W=d['map']['width'];H=d['map']['height'];c=d['map']['cells']
    cost={0:2,1:1,2:3}; fc={0:1,1:2,2:2}; N=d['daySteps'][day]; FUEL=d['fuelLimits']
    def nb(p):
        r,q=divmod(p,W)
        for dc,dr in ([(0,-1),(1,-1),(1,0),(1,1),(0,1),(-1,0)] if r%2==0 else [(-1,-1),(0,-1),(1,0),(0,1),(-1,1),(-1,0)]):
            rr,cc=r+dr,q+dc
            if 0<=rr<H and 0<=cc<W and c[rr][cc]!=3: yield rr*W+cc
    def pareto(s):
        F={s:[(0,0)]}; h=[(0,0,s)]
        while h:
            t,f,p=heapq.heappop(h)
            if (t,f) not in F.get(p,[]): continue
            r,q=divmod(p,W)
            for n in nb(p):
                nt,nf=t+cost[c[r][q]],f+fc[c[r][q]]
                if nt>N or nf>FUEL: continue
                L=F.setdefault(n,[])
                if any(a<=nt and b<=nf for a,b in L): continue
                L[:]=[(a,b) for a,b in L if not(nt<=a and nf<=b)]+[(nt,nf)]
                heapq.heappush(h,(nt,nf,n))
        return F
    spots=d['spots']; brands=sorted(set(s['brand'] for s in spots)); bi={b:i for i,b in enumerate(brands)}
    SP=[pareto(s['pos']) for s in spots]
    def add(front,leg):
        out=[]
        for t0,f0 in front:
            for t,f in leg:
                if t0+t<=N and f0+f<=FUEL: out.append((t0+t,f0+f))
        out=sorted(set(out)); res=[]
        for x in out:
            if not res or x[1]<res[-1][1]: res.append(x)
        return res
    def dominated(new,old):
        return all(any(a<=t and b<=f for a,b in old) for t,f in new)
    def maximal(ms):
        keep=[]
        for m in sorted(set(ms),key=lambda m:-bin(m).count('1')):
            if not any(m|k==k for k in keep): keep.append(m)
        return keep
    per=[]
    for a in d['agents']:
        A=pareto(a); best={}; prev={}; frontier=[]
        for j,s in enumerate(spots):
            fr=add([(0,0)],A.get(s['pos'],[]))
            if fr: st=(j,1<<bi[s['brand']]); best[st]=fr; prev[st]=None; frontier.append(st)
        while frontier:
            nxt=[]
            for st in frontier:
                cur,mask=st
                for j,s in enumerate(spots):
                    b=1<<bi[s['brand']]
                    if mask&b: continue
                    fr=add(best[st],SP[cur].get(s['pos'],[]))
                    if not fr: continue
                    ns=(j,mask|b)
                    if ns in best and dominated(fr,best[ns]): continue
                    merged=sorted(set(best.get(ns,[])+fr)); res=[]
                    for x in merged:
                        if not res or x[1]<res[-1][1]: res.append(x)
                    best[ns]=res; prev[ns]=st; nxt.append(ns)
            frontier=list(dict.fromkeys(nxt))
        wit={0:([],[(0,0)])}
        for st in best:
            if st[1] not in wit:
                r=[];x=st
                while x is not None: r.append(spots[x[0]]['pos']); x=prev[x]
                wit[st[1]]=(r[::-1],best[st])
        per.append((maximal(list(wit)),wit))
    n=len(d['agents'])
    print(f.split('/')[-1],'day',day,'steps',N,'fuel',FUEL,'series',len(brands))
    for slot in list(range(n))+[None]:
        pats=[i for i in range(n) if i!=slot]; unions={0:()}
        for i in pats:
            nu={}
            for u,w in unions.items():
                for m in per[i][0]:
                    if (u|m) not in nu: nu[u|m]=w+(m,)
            unions={k:nu[k] for k in maximal(list(nu))}
        bu=max(unions,key=lambda u:bin(u).count('1'))
        print('  supply slot',slot,'max series without any refuel:',bin(bu).count('1'),'missing',[brands[b] for b in range(len(brands)) if not bu>>b&1])
        for i,m in zip(pats,unions[bu]): print('     agent',i,'spots',per[i][1][m][0],'(time,fuel) options',per[i][1][m][1])
for f in sys.argv[1:]: run(f)
