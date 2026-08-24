#!/usr/bin/env python3
"""Crawl the public Tes taxonomy API for target markets. No auth, paced, bounded."""
import json, time, urllib.request, pathlib, sys
BASE="https://www.tes.com/taxonomy/v4"
OUT=pathlib.Path(__file__).resolve().parent.parent/"docs/design/data"
OUT.mkdir(parents=True, exist_ok=True)
FULL=["GB","NZ"]; REF=["US","AU","IE","CA"]
budget=[2000]
def get(path):
    if budget[0]<=0: raise SystemExit("request budget exhausted")
    budget[0]-=1
    req=urllib.request.Request(f"{BASE}/{path}", headers={"User-Agent":"listing-sync-taxonomy-probe/0.1"})
    with urllib.request.urlopen(req, timeout=25) as r:
        return json.loads(r.read().decode())
    time.sleep(0.1)
def node_brief(n):
    return {"id":n.get("id"),"description":n.get("description"),
            "leafDepth":(n.get("leaf","").split("|",1)[0] if isinstance(n.get("leaf"),str) else None),
            "phases":n.get("phases",[]),"mapTo":n.get("mapTo",[]),"parentId":n.get("parentId")}
for c in FULL:
    root=get(c); subs=root.get("children",[]) or []
    out=[]
    for s in subs:
        b=node_brief(s)
        try:
            sn=get(f"{c}/{s['id']}"); topics=sn.get("children",[]) or []
            b["topics"]=[node_brief(t) for t in topics]
        except Exception as e:
            b["topics"]=[]; b["_err"]=str(e)
        out.append(b); time.sleep(0.08)
    (OUT/f"tes-taxonomy-{c}.json").write_text(json.dumps(
        {"_source":f"GET {BASE}/{c} crawl 2026-08-25, subjects+topics (subtopics omitted)","country":c,
         "subjectCount":len(out),"topicCount":sum(len(x['topics']) for x in out),"subjects":out}, indent=1))
    print(f"{c}: {len(out)} subjects, {sum(len(x['topics']) for x in out)} topics")
refs={}
for c in REF:
    try:
        root=get(c); subs=root.get("children",[]) or []
        refs[c]={"subjectCount":len(subs),"subjects":[node_brief(s) for s in subs]}
        print(f"{c} (ref): {len(subs)} subjects")
    except Exception as e:
        refs[c]={"_err":str(e)}
    time.sleep(0.1)
(OUT/"tes-taxonomy-refs.json").write_text(json.dumps(
    {"_source":f"GET {BASE}/{{country}} subject roots only, 2026-08-25","countries":refs}, indent=1))
print("done; requests used:", 2000-budget[0])
