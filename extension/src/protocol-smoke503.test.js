import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import {webcrypto} from 'node:crypto';
import {signedGet,signedFetch} from './protocol.js';

test('isolated smoke HTTP503 preserves the synthetic paired extension token',async()=>{
  globalThis.crypto??=webcrypto;const previous=globalThis.chrome;const storage={bridgeToken:'synthetic-existing-token'};let removed=false;
  globalThis.chrome={runtime:{id:'synthetic-existing-extension'},storage:{local:{get:async()=>({...storage}),remove:async()=>{removed=true;delete storage.bridgeToken;}}}};
  const server=http.createServer((request,response)=>{if(request.url==='/v1/health'){response.setHeader('Content-Type','application/json');response.end(JSON.stringify({ready:true,takeover_enabled:false,bt_magnet_enabled:false}));}else {response.writeHead(503);response.end('isolated smoke');}});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  try {
    const base=`http://127.0.0.1:${server.address().port}`;
    assert.equal((await signedGet(base+'/v1/tasks/recent')).status,503);
    assert.equal((await signedFetch(base+'/v1/tasks',{url:'https://example.invalid/synthetic'})).status,503);
    assert.equal(removed,false);assert.equal(storage.bridgeToken,'synthetic-existing-token');
  } finally {globalThis.chrome=previous;await new Promise(resolve=>server.close(resolve));}
});
