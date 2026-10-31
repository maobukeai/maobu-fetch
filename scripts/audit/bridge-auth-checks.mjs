// Serialized into the isolated extension page; credentials never leave that page.
export async function authenticateBridge(config) {
  const protocol=await import('./protocol.js');
  await chrome.storage.local.set({intercept:false});
  const pair=(code)=>fetch(protocol.API+'/v1/pair',{method:'POST',headers:{'Content-Type':'application/json',Origin:location.origin},body:JSON.stringify({code,extension_id:chrome.runtime.id})});
  const wrong=await pair(config.code==='000000'?'111111':'000000');
  if(wrong.status!==401)throw new Error('Wrong pairing code must be rejected');
  const response=await chrome.runtime.sendMessage({type:'pair',code:config.code});if(response?.ok!==true)throw new Error('Real background pairing failed');
  const {bridgeToken}=await chrome.storage.local.get('bridgeToken');if(typeof bridgeToken!=='string')throw new Error('Pair response invalid');
  const paired={token:bridgeToken};
  const replay=await pair(config.code);if(replay.status!==401)throw new Error('Consumed pairing code replay was accepted');
  const raw=async(body,timestamp,signedBody=body,extension=chrome.runtime.id)=>fetch(protocol.API+'/v1/tasks',{method:'POST',headers:{'Content-Type':'application/json',Origin:location.origin,'X-Luma-Extension':extension,'X-Luma-Timestamp':timestamp,'X-Luma-Signature':await protocol.signature(paired.token,timestamp,signedBody)},body});
  const body=JSON.stringify({url:config.source_url,start_paused:true});
  const origin=await raw(body,String(Date.now()),body,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
  const expired=await raw(body,String(Date.now()-10*60*1000));
  const modified=await raw(body,String(Date.now()),body+' ');
  if(origin.status!==403||expired.status!==401||modified.status!==401)throw new Error('Origin, timestamp or tampered body was accepted');
  window.bridgeFixture={protocol,raw,token:paired.token};
  return {wrong_code:wrong.status,pair:true,consumed_code:replay.status,origin_binding:origin.status,expired_timestamp:expired.status,tampered_body:modified.status};
}

export async function verifyRevocation() {
  const {protocol,raw}=window.bridgeFixture;
  const revoked=await protocol.signedGet('/v1/tasks/recent');
  if(revoked.status!==401)throw new Error('Revoked token was accepted');
  if((await chrome.storage.local.get('bridgeToken')).bridgeToken)throw new Error('Confirmed revocation did not clear extension token');
  let limited=false;
  for(let index=0;index<121;index++) {
    const response=await raw('{}',String(Date.now()));
    if(response.status===429){limited=true;break;}
    if(response.status!==401)throw new Error('Unexpected revoked request status');
  }
  if(!limited)throw new Error('Real Rust bridge rate limiter was bypassed');
  await chrome.storage.local.set({bridgeToken:'synthetic-offline-only'});
  return {revoked:revoked.status,rate_limit:429,token_removed:true};
}
