export async function exerciseTasks(config) {
  const {protocol}=window.bridgeFixture;
  const {performBridgeTaskAction}=await import('./task-action.js');
  const background=await import('./background.js');
  const tabs=await chrome.tabs.query({});const tab=tabs.find(tab=>tab.url===new URL('/',config.source_url).href);
  if(!tab)throw new Error('Synthetic source tab missing');
  await chrome.cookies.set({url:config.source_url,name:'bridge_session',value:'SYNTHETIC'});
  const headers=await background.getTabDownloadHeaders(tab,config.source_url);
  if(!headers.Cookie?.includes('bridge_session=SYNTHETIC'))throw new Error('Real cookie API did not provide target credential');
  const send=async(path,payload)=>{const response=await protocol.signedFetch(path,payload);if(!response.ok)throw new Error('Bridge request rejected '+response.status);if(path==='/v1/tasks'&&response.status!==201)throw new Error('Task creation must return 201');return response.json();};
  const action=async(id,action)=>{const result=await performBridgeTaskAction(id,action,protocol.signedFetch);if(!result.success||result.paused_instead)throw new Error('Bridge '+action+' failed');};
  const wait=async(id,status)=>{
    const deadline=Date.now()+12000;
    let last=null;
    while(Date.now()<deadline) {
      const response=await protocol.signedGet('/v1/tasks/recent');if(!response.ok)throw new Error('Recent tasks rejected '+response.status);
      const task=(await response.json()).tasks.find(task=>task.id===id);
      last=task?{status:task.status,error:task.error??null,progress:task.progress}: {status:'missing-from-recent'};
      if(task?.status===status)return task;
      if(task?.status==='failed'||task?.status==='remote_changed')throw new Error('Bridge download failed');
      await new Promise(resolve=>setTimeout(resolve,250));
    }
    throw new Error('Bridge task did not reach '+status+'; observed='+JSON.stringify(last));
  };
  const base={destination:config.destination,headers,connection_count:1,start_paused:true,user_edited_file_name:true,collision_policy:'rename'};
  const cancelled=await send('/v1/tasks',{...base,url:config.slow_url,file_name:'cancelled.bin'});
  if(cancelled.status!=='paused')throw new Error('Start paused was ignored');
  await action(cancelled.id,'resume');
  if(config.resume_probe_delay_ms)await new Promise(resolve=>setTimeout(resolve,config.resume_probe_delay_ms));
  await wait(cancelled.id,'downloading');
  await action(cancelled.id,'pause');await wait(cancelled.id,'paused');
  await action(cancelled.id,'cancel');await wait(cancelled.id,'cancelled');
  const completed=await send('/v1/tasks',{...base,url:config.source_url,file_name:'complete.bin',connection_count:4,expected_checksum:config.expected_sha256});
  await action(completed.id,'resume');await wait(completed.id,'completed');
  return {cancelled_task:cancelled.id,completed_task:completed.id,created_status:201,pause:true,resume:true,cancel:true,completed:true};
}

export async function downloadWhileOffline(config) {
  const {protocol}=window.bridgeFixture;
  let offline=false;try{await protocol.signedGet('/v1/tasks/recent');}catch{offline=true;}
  if(!offline)throw new Error('Fixture bridge must be offline');
  const background=await import('./background.js');
  const tab=(await chrome.tabs.query({})).find(tab=>tab.url===new URL('/',config.source_url).href);
  const notifications=[];
  await background.handleContextMenuDownload({linkUrl:config.source_url},tab,{notify:title=>notifications.push(title),focusDesktop:()=>{}});
  const deadline=Date.now()+10000;
  while(Date.now()<deadline) {
    const downloads=await chrome.downloads.search({url:config.source_url});
    const download=downloads.find(item=>item.byExtensionId===chrome.runtime.id);
    if(download?.state==='complete') {
      if(!notifications.some(title=>title.includes('浏览器')))throw new Error('Offline fallback was not reported truthfully');
      return {state:download.state,filename:download.filename};
    }
    if(download?.state==='interrupted')throw new Error('Browser fallback was interrupted');
    await new Promise(resolve=>setTimeout(resolve,100));
  }
  throw new Error('Actual browser fallback did not finish');
}
