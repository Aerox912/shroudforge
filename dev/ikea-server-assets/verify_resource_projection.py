"""Independent checks over bounded typed readback, never original game inputs.

Reports asset semantics only. Native mods, content payloads, multiplayer and
runtime behavior require their separate checks and the controlled server trial.
"""
import argparse
import csv
import hashlib
import io
import json
import math
import pathlib
import re

RECIPE_SHA='3bd7950844b905ddccd64907bc179584c91b22bd2a1b446304d10cd101e4b1c6'
CATALOG_SHA='4f2a4201e2e22a36c6c8c87af4c5ff2262e541f66a6227e1a380aee17f03adc8'
def require(ok,message):
    if not ok:raise ValueError(message)
def sha(b):return hashlib.sha256(b).hexdigest()
def number(v):
    while isinstance(v,dict):
        require('value' in v or '$value' in v,'numeric wrapper missing')
        v=v['value'] if 'value' in v else v['$value']
    require(isinstance(v,(int,float)) and not isinstance(v,bool) and math.isfinite(v),'numeric value required')
    return v
def value(v):return v.get('$value',v) if isinstance(v,dict) else v
def components(v):
    result={}
    for c in v.get('components',[]):
        name=c.get('$typeName');require(name is not None and name not in result,'missing/duplicate component type')
        result[name]=value(c)
    return result
def close(a,b):return math.isclose(number(a),number(b),rel_tol=2e-6,abs_tol=1e-6)
def fnv(a):
    h=2166136261
    for v in a:h=((h^(int(number(v))&0xffffffff))*16777619)&0xffffffff
    return h
def load_recipe(path):
    b=path.read_bytes();require(sha(b)==RECIPE_SHA,'recipe differs');r=json.loads(b)
    def source(name):
        p=r['effectiveFiles'][name];b=pathlib.Path(p['path']).read_bytes()
        require(len(b)==p['bytes'] and sha(b)==p['sha256'],'recipe source differs: '+name)
        return b.decode('utf-8-sig')
    god=source('mods/godmode_spawn_variants/src/mod.lua')
    bases=dict(re.findall(r'\["(Enemy_[^"]+)"\]\s*=\s*"([0-9a-f-]{36})"',god));require(len(bases)==33,'base count')
    main=source('mods/aldrich-witcher-signs/src/mod.lua');part=main.split('local ID = {',1)[1].split('\n}',1)[0]
    ids=dict(re.findall(r'(\w+)\s*=\s*"([0-9a-f-]{36})"',part))
    for key,guid in re.findall(r'(\w+)\s*=\s*"([0-9a-f-]{36})"',source('mods/aldrich-witcher-signs/src/identities.lua')):
        require(key not in ids or ids[key]==guid,'Witcher identity conflict');ids[key]=guid
    keys=[]
    for family in ['aard','igni','yrden','quen','axii','alzur']:
        for rank in range(3 if family=='alzur' else 1,8):
            keys.append(family if rank==1 or family=='alzur' and rank==3 else family+'2' if rank==3 else family+'_t'+str(rank))
        keys.append(family+'3')
    require(len(keys)==46,'sign count')
    keys+=['swallow','thunderbolt','tawny','cat','whale','raffard','full_moon','wolf','petri','white_honey']
    return bases,{key:(ids['item_'+key],ids['recipe_'+key]) for key in keys}

def verify(rows,bases,witcher,catalog):
    grouped={};seen=set()
    for r in rows:
        key=(r['type'],r['guid'],r['part']);require(key not in seen,'duplicate resource');seen.add(key)
        grouped.setdefault(r['type'],[]).append(r)
    checks={};items=grouped['keen::ItemInfo'];templates=grouped['keen::ecs::TemplateResource']
    by_item={r['guid']:r['current'] for r in items if r['part']==0 and r['current'] is not None}
    by_template={r['guid']:r['current'] for r in templates if r['part']==0 and r['current'] is not None}
    stack=single=fog=0
    for r in items:
        a,b=r['baseline'],r['current']
        if a is None:continue
        require(b is not None,'existing item removed')
        if isinstance(a.get('maxStackSize'),(int,float)):
            n=a['maxStackSize'];require(b['maxStackSize']==(65535 if n>1 else n),'stack/single preservation '+r['guid'])
            stack+=n>1;single+=n<=1
        equipment=a.get('equipment')
        if equipment is not None and 'Material' not in a.get('debugName',''):
            now=b['equipment'];require(now['allowPlacementBelowFog'] is True,'fog placement '+r['guid'])
            if equipment.get('checkInhibitBuild') in ['Strict','Lenient']:require(now['checkInhibitBuild']=='None','fog inhibit '+r['guid'])
            fog+=1
    require(stack>0 and single>0 and fog>0,'empty item effect checks');checks['stack']={'stackable':stack,'singlePreserved':single};checks['shroudBuilding']=fog
    buff_count=0;rested_ui=0
    for r in grouped['keen::BuffTypeCollectionResource']:
        current={number(b['buffTypeId']):b for b in r['current']['buffs']}
        for b in r['baseline']['buffs']:
            n=current[number(b['buffTypeId'])]
            if b['applyType']=='ReplaceAtEnd':require(n['applyType']=='Replace','buff refresh');buff_count+=1
        if 3451821184 in current:
            b=current[3451821184];require(b['slot']=='Rested' and b['impact']=='91715e7c-46b6-4dae-9058-09780481a9bb' and b['uiValues']==[],'Rested UI');rested_ui+=1
    require(buff_count>0 and rested_ui==1,'buff coverage');checks['buffRefresh']=buff_count
    scenes=[r for r in grouped['keen::SceneResource'] if r['baseline'] and r['baseline'].get('ibl')=='9743d1a0-007d-4a8a-9088-82d34ce7a697' and r['baseline'].get('noBuildZones') is not None]
    require(scenes and all(r['current']['noBuildZones']==[] for r in scenes),'build zones');checks['buildZones']=len(scenes)
    sources=[v for v in by_template.values() if v['name']=='Prop_Decoration_T1_Storage_24_Magic'];require(len(sources)==1,'magic source')
    magic='keen::ecs::InventoryCraftingStock';inventory='keen::ecs::InventorySetup';source_component=components(sources[0])[magic]
    factory=furniture=0
    for r in templates:
        a,b=r['baseline'],r['current']
        if a is None:continue
        name=a['name'];c=components(a)
        if inventory not in c:continue
        is_factory='Factory' in name
        is_furniture='Prop_Decoration_' in name and 'Storage' in name and all(x not in name for x in ['Magic','Loot','DEPRECATED'])
        if is_factory or is_furniture:
            n=components(b);require(magic in n,'missing magic storage '+name)
            if magic not in c:require(n[magic]==source_component,'magic clone differs '+name)
            factory+=is_factory;furniture+=is_furniture
    require(factory>0 and furniture>0,'magic target coverage');checks['magicFactories']=factory;checks['magicFurniture']=furniture
    picks=['59dba0f3-81b8-4ac5-88d7-ba97f4d2928c','46ea9b46-445a-4e36-bf84-20cab7585f48','70ecc7fb-6ed1-416c-a51e-0362b2ebdd1a','c03c05ce-4cc0-40fd-b201-58d05c730767','2e4b3166-4960-48b1-ad1f-f3bb9d0d981a','aaff198b-9139-47f5-8e41-b1ce0262abc8','aee6b3ba-adeb-4264-b117-25f974a08cb8']
    for guid in picks:require(close(by_item[guid]['equipment']['maxDistance'],10),'pickaxe reach '+guid)
    checks['veinAssetReach']=7
    hooks={'70096018-ed26-4697-a13d-0f2f201760a3','3b8c8e70-a653-44d1-8061-7cb474c540da','281a919a-e117-47b2-bdcb-4453834e3482'}
    for guid in hooks:
        rr=[r for r in items if r['guid']==guid and r['part']==0];require(len(rr)==1,'hook identity')
        pair=[]
        for side in ['baseline','current']:
            configs={number(value(e)['configId']):number(value(e)['value']) for e in rr[0][side]['impactValues']['simple']}
            pair.append(configs)
        require(pair[0][4151084092]>0 and close(pair[1][4151084092],2*pair[0][4151084092]),'pull distance')
        require(pair[1][3078764024]==pair[0][3078764024]>0,'swing changed')
    checks['grapplingPull']=3
    impact=[r for r in grouped['keen::ImpactProgram'] if r['guid']=='91715e7c-46b6-4dae-9058-09780481a9bb' and r['part']==0];require(len(impact)==1,'Rested program')
    p=impact[0]['current']
    for field,length,digest in [('code',253,1438511556),('codeShutdown',189,3442588861),('data',1104,3403271353)]:require(len(p[field])==length and fnv(p[field])==digest,'Rested '+field)
    require(len(p['dataLayout'])==69 and p['stackSize']==256 and p['usedStreams']==[] and p['queryCallIds']==[],'Rested program metadata');checks['restedProgram']=True
    recipes={};timed=0
    for r in grouped['keen::RecipeRegistryResource']:
        now={number(x['recipeId']):x for x in r['current']['recipes']}
        for recipe in r['current']['recipes']:
            guid=recipe['recipeGuid'];require(guid not in recipes,'duplicate recipe GUID');recipes[guid]=recipe
        for old in r['baseline']['recipes']:
            n=number(old['craftingDuration']);cur=now[number(old['recipeId'])]
            expected=max(1,math.floor(n/20+0.5)) if n>0 else n
            require(number(cur['craftingDuration'])==expected,'workshop duration');timed+=n>0
    require(timed>0,'workshop coverage');checks['workshopTimedRecipes']=timed
    variants=0
    for name,guid in bases.items():
        base=by_template[guid];require(base['name']==name,'Godmode base identity');bc=components(base)
        for i,(preset,health,damage) in enumerate([('x3',3,1.5),('x10',10,2.5),('x40',40,5)],1):
            parts=guid.split('-');vg=f'{parts[0]}-{parts[1]}-4182-{i:04x}-{parts[4]}'
            variant=by_template[vg];require(variant['name']==name+'@'+preset,'Godmode variant name');vc=components(variant)
            if 'keen::ecs::EnemyScaling' in bc:
                a=bc['keen::ecs::EnemyScaling']['enemyLevelScaling']['healthScaleFactor'];b=vc['keen::ecs::EnemyScaling']['enemyLevelScaling']['healthScaleFactor'];require(close(b,number(a)*health),'Godmode health')
            if 'keen::ecs::DamageMod' in bc:require(close(vc['keen::ecs::DamageMod']['dataStorage'][1],number(bc['keen::ecs::DamageMod']['dataStorage'][1])*damage),'Godmode damage')
            variants+=1
    require(variants==99,'Godmode variant coverage');checks['godmodeVariants']=variants
    registries=grouped['keen::ItemRegistryResource'];registered={str(g) for r in registries for g in r['current']['itemRefs']}
    for key,(item_guid,recipe_guid) in witcher.items():
        require(item_guid in by_item and item_guid in registered,'Witcher item registration '+key)
        require(any(str(x['itemRef'])==item_guid for x in recipes[recipe_guid]['output']),'Witcher recipe output '+key)
    require(len(witcher)==56,'Witcher identity coverage');checks['witcherItemRecipeRegistration']=56
    if catalog is not None:
        require(sha(catalog)==CATALOG_SHA,'catalog differs');catalog_rows=list(csv.DictReader(io.StringIO(catalog.decode('utf-8-sig'))));require(catalog_rows,'empty catalog')
        for row in catalog_rows:
            require(row['guid'] in by_template and by_template[row['guid']]['name']==row['name'],'catalog template identity '+row['guid'])
            c=components(by_template[row['guid']]);require(c['keen::ecs::Faction']['faction']==row['faction'],'catalog faction '+row['guid'])
            require(('keen::ecs::Boss' in c or 'keen::ecs::BossHealthBar' in c)==(row['boss']=='true'),'catalog boss flag '+row['guid'])
        checks['godmodeCatalogRows']=len(catalog_rows)
    return checks

def main():
    p=argparse.ArgumentParser();p.add_argument('report');p.add_argument('report_sha256');p.add_argument('recipe');p.add_argument('output');p.add_argument('--catalog');a=p.parse_args()
    path=pathlib.Path(a.report);require(path.stat().st_size<=256*1024*1024,'report extent');raw=path.read_bytes();require(sha(raw)==a.report_sha256,'report hash')
    lines=raw.splitlines();require(2<=len(lines)<=100002,'report line count');header=json.loads(lines[0]);complete=json.loads(lines[-1])
    require(header['kind']=='header' and header['strictSchemaQualification'] is False and header['registrySha256']=='fe74476f286d01fbd677b97abc6472ebb886313a7358ce0149cf1c6670bce4f3','header')
    require(complete['kind']=='complete' and complete['status']=='BOUNDED_RESOURCE_PROJECTION_PENDING_SEMANTIC_REVIEW','incomplete reader')
    rows=[json.loads(line) for line in lines[1:-1]];require(all(r['kind']=='resource' for r in rows),'unexpected record')
    bases,witcher=load_recipe(pathlib.Path(a.recipe));catalog=pathlib.Path(a.catalog).read_bytes() if a.catalog else None
    checks=verify(rows,bases,witcher,catalog)
    report=dict(status='SERVER_ASSET_CHECKS_PASSED_PENDING_INDEPENDENT_REVIEW',reportSha256=a.report_sha256,readerHeader=header,readerCompletion=complete,recipeSha256=RECIPE_SHA,checks=checks,catalogChecked=catalog is not None,nativeCompatibilityAccepted=False,gameplayAccepted=False,limits=['Resource projections do not decode localization or other content payloads','Witcher check covers item/recipe registration, not every effect graph or runtime outcome','Flight and Gifting require independent native cohort acceptance'],liveChanges=False)
    with pathlib.Path(a.output).open('x',encoding='utf-8') as f:json.dump(report,f,indent=2)
    print(json.dumps(report))
if __name__=='__main__':main()
