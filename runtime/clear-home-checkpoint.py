#!/usr/bin/env python3
# Remove only derived Home checkpoints; never inputs or global CRIU state.
import os,sys,json,re
ROOTS=['/tmp/checkpoint/com.webos.app.home','/mnt/lg/cmn_data/checkpoint/com.webos.app.home']
assert not os.path.exists('/tmp/criu_restoring'),'Restore active: refuse cache change'
metadata={'stats-restore','stats-dump','img_size','pgid','MD5SUM','starfish-release','flock','fault_pages.dat','invalid_img'}
plan=[]
for root in ROOTS:
 if not os.path.exists(root):continue
 assert os.path.isdir(root) and not os.path.islink(root) and os.path.realpath(root)==root,root
 names=os.listdir(root)
 for name in names:
  p=root+'/'+name
  assert not os.path.isdir(p),(p,'Unexpected directory')
  assert name in metadata or re.fullmatch(r'[A-Za-z0-9_.-]+\.img(?:\.ori|\.old)?',name),(p,'Unknown checkpoint file')
 plan.append({'root':root,'files':names})
print(json.dumps({'plan':plan,'apply':sys.argv[1:]==['--apply']}),flush=True)
if sys.argv[1:]==['--apply']:
 assert os.path.isfile('/var/lib/lg-native/checkpoint-backup.json')
 for group in plan:
  for name in group['files']:os.unlink(group['root']+'/'+name)
  os.rmdir(group['root']);assert not os.path.exists(group['root'])
 print('Only Home checkpoint roots removed; inputcommon and global CRIU untouched',flush=True)
else:assert sys.argv[1:]==['--dry-run'],'Explicit mode required'
