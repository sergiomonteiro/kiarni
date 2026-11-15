// Real Nginx + OpenSSL + product integration. Development-only dependencies.
import test from 'node:test';
import assert from 'node:assert/strict';
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdtemp,readFile,writeFile,readdir,mkdir,chmod,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createServer,connect} from 'node:net';
import https from 'node:https';
import http from 'node:http';
import {createHash,createPublicKey} from 'node:crypto';
import {createInterface} from 'node:readline';
import {setTimeout as delay} from 'node:timers/promises';
import {chromium} from '../spikes/transport/node_modules/playwright/index.mjs';
const run=promisify(execFile);
async function port() {const server=createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));return port;}
async function stop(child) {
  if (!child || child.exitCode!==null || child.signalCode!==null) return;
  const done=new Promise(r=>child.once('exit',r));child.kill('SIGTERM');
  let timer;await Promise.race([done,new Promise(r=>{timer=setTimeout(()=>{child.kill('SIGKILL');r();},5000);})]);clearTimeout(timer);await done;
}
function capture(child) {let output='';child.stderr.on('data',bytes=>{output+=bytes.toString('utf8');if(output.length>1024*1024) child.kill('SIGKILL');});return ()=>output;}
async function until(predicate) {for(let i=0;i<200;i++){if(await predicate())return;await delay(20);}throw new Error('fixture readiness deadline');}

test('HTTPS proxy preserves authentication, origin checks, bounded SSE and private telemetry',{timeout:90000},async()=>{
  const temp=await mkdtemp(join(tmpdir(),'kiarni-https-'));await chmod(temp,0o700);
  const backendPort=await port(),frontendPort=await port();
  const authority=`kiarni.test:${frontendPort}`,origin=`https://${authority}`;
  const nginxBin=process.env.KIARNI_TEST_NGINX || 'nginx';
  let backend,nginx,browser;const streams=[];
  let backendLog=()=>'',nginxLog=()=>'';
  try {
    await run('openssl',['req','-x509','-newkey','rsa:2048','-nodes','-keyout',join(temp,'ca.key'),'-out',join(temp,'ca.crt'),'-days','2','-subj','/CN=Kiarni test CA']);
    await run('openssl',['req','-new','-newkey','rsa:2048','-nodes','-keyout',join(temp,'server.key'),'-out',join(temp,'server.csr'),'-subj','/CN=kiarni.test']);
    await writeFile(join(temp,'extensions.cnf'),'subjectAltName=DNS:kiarni.test\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n',{mode:0o600});
    await run('openssl',['x509','-req','-in',join(temp,'server.csr'),'-CA',join(temp,'ca.crt'),'-CAkey',join(temp,'ca.key'),'-CAcreateserial','-out',join(temp,'server.crt'),'-days','2','-sha256','-extfile',join(temp,'extensions.cnf')]);
    for(const name of ['ca.key','server.key'])await chmod(join(temp,name),0o600);
    const ca=await readFile(join(temp,'ca.crt'));
    const replacements={PREFIX:temp,LISTEN:`127.0.0.1:${frontendPort}`,SERVER_NAME:'kiarni.test',AUTHORITY:authority,UPSTREAM:`127.0.0.1:${backendPort}`,CERT:join(temp,'server.crt'),KEY:join(temp,'server.key')};
    let config=await readFile('deploy/nginx/kiarni.conf.template','utf8');
    for(const [name,value] of Object.entries(replacements)){assert.match(value,/^[A-Za-z0-9_./:-]+$/);config=config.replaceAll(`@${name}@`,value);}
    assert.ok(!/@[A-Z_]+@/.test(config));await writeFile(join(temp,'nginx.conf'),config,{mode:0o600});
    for(const name of ['body','proxy'])await mkdir(join(temp,name),{mode:0o700});
    await writeFile(join(temp,'access.log'),'',{mode:0o600});
    await run(nginxBin,['-t','-p',temp+'/','-c',join(temp,'nginx.conf'),'-e',join(temp,'startup.log')]);
    backend=spawn(resolve('target/debug/kiarni'),['serve','--data-dir',join(temp,'data'),'--bind',`127.0.0.1:${backendPort}`,'--public-origin',origin,'--heartbeat-ms','1000'],{env:{},stdio:['ignore','ignore','pipe']});backendLog=capture(backend);
    await until(async()=>{try{await new Promise((res,rej)=>{const socket=connect(backendPort,'127.0.0.1',()=>{socket.destroy();res();});socket.once('error',rej);});return true;}catch{return false;}});
    const files=await readdir(join(temp,'data.credentials'));assert.equal(files.length,1);const token=await readFile(join(temp,'data.credentials',files[0]),'utf8');
    nginx=spawn(nginxBin,['-p',temp+'/','-c',join(temp,'nginx.conf'),'-e',join(temp,'startup.log'),'-g','daemon off;'],{stdio:['ignore','ignore','pipe']});nginxLog=capture(nginx);
    const options=(path,extra={})=>({hostname:'127.0.0.1',port:frontendPort,servername:'kiarni.test',path,agent:false,ca,rejectUnauthorized:true,headers:{Host:authority,...extra.headers},...Object.fromEntries(Object.entries(extra).filter(([key])=>key!=='headers'))});
    const request=(path,extra={},body)=>new Promise((res,rej)=>{
      const headers={...extra.headers};if(body!==undefined){if(!headers['Transfer-Encoding'])headers['Content-Length']=Buffer.byteLength(body);headers['Content-Type']??='application/json';}
      const req=https.request(options(path,{...extra,headers}),response=>{const protocol=response.socket.getProtocol();let text='';response.setEncoding('utf8');response.on('data',chunk=>{text+=chunk;if(Buffer.byteLength(text)>5*1024*1024)req.destroy(new Error('oversized fixture response'));});response.on('end',()=>res({status:response.statusCode,headers:response.headers,text,protocol}));response.on('error',rej);});
      req.setTimeout(5000,()=>req.destroy(new Error('HTTPS fixture deadline')));req.on('error',rej);req.end(body);
    });
    const auth={Authorization:`Bearer ${token}`};
    await until(async()=>{try{return (await request('/')).status===200;}catch{return false;}});
    const baseline=await request('/');assert.equal(baseline.status,200);assert.match(baseline.headers['content-security-policy'],/default-src 'none'/);assert.equal(baseline.headers['strict-transport-security'],'max-age=31536000');
    assert.ok(['TLSv1.2','TLSv1.3'].includes(baseline.protocol));
    for(const version of ['TLSv1.2','TLSv1.3'])assert.equal((await request('/',{minVersion:version,maxVersion:version})).protocol,version);
    await assert.rejects(request('/',{ca:undefined}));
    await assert.rejects(request('/',{servername:'wrong.test'}));
    const plaintext=await new Promise((res,rej)=>{const req=http.get({host:'127.0.0.1',port:frontendPort,path:'/',headers:{Host:authority},agent:false},r=>{r.resume();r.on('end',()=>res(r.statusCode));});req.setTimeout(3000,()=>req.destroy());req.on('error',rej);});assert.equal(plaintext,400);
    for(const path of ['/v1/health/live','/v1/status','/v1/metrics','/v1/errors/recent','/v1/collections/orders/changes']){
      const response=await request(path);assert.equal(response.status,401);assert.match(response.headers['x-request-id'],/^req_[a-f0-9]{32}$/);
    }
    assert.equal((await request('/v1/status',{headers:{...auth,Host:'wrong.test'}})).status,421);
    for(const value of ['https://wrong.test',`http://${authority}`,'null',''])assert.equal((await request('/v1/status',{headers:{...auth,Origin:value}})).status,403);
    assert.equal((await request('/v1/status',{headers:{Forwarded:`for=127.0.0.1;host=${authority};proto=https`,'X-Forwarded-Proto':'https'}})).status,401);
    assert.equal((await request('/v1/status',{headers:{...auth,Origin:origin,Forwarded:'host=wrong.test;proto=http','X-Forwarded-Host':'wrong.test'}})).status,200);
    assert.equal((await request('/v1/collections',{method:'POST',headers:{...auth,Origin:origin}},'{"name":"orders"}')).status,201);
    const marker='private_document_marker_kiarni';
    const document=`{"_id":"doc","amount":9007199254740993,"payload":"${marker}"}`;
    assert.equal((await request('/v1/collections/orders/documents',{method:'POST',headers:{...auth,Origin:origin}},document)).status,201);
    const query=await request('/v1/collections/orders/query',{method:'POST',headers:{...auth,Origin:origin}},'{"from":"orders","limit":1}');assert.equal(query.status,200);assert.ok(query.text.includes('9007199254740993'));
    assert.equal((await request('/v1/collections/orders/documents',{method:'POST',headers:{...auth,'Transfer-Encoding':'chunked'}},'{"_id":"chunked"}')).status,201);
    const metrics=await request('/v1/metrics',{headers:auth});assert.equal(metrics.status,200);assert.match(metrics.text,/kiarni_http_requests_total/);
    const subscribe=(slow=false)=>new Promise((res,rej)=>{const req=https.request(options('/v1/collections/orders/changes',{headers:{...auth,Origin:origin,Accept:'text/event-stream'}}),response=>{
      try{assert.equal(response.statusCode,200);assert.match(response.headers['content-type'],/text\/event-stream/);}catch(e){req.destroy();rej(e);return;}
      response.on('error',()=>{});
      if(slow){response.pause();const stream={req,response,close(){req.destroy();response.destroy();}};streams.push(stream);res(stream);return;}
      const lines=createInterface({input:response,crlfDelay:Infinity})[Symbol.asyncIterator]();
      const stream={req,response,async frame(){let text='';for(;;){const next=await lines.next();assert.ok(!next.done,'SSE closed unexpectedly');text+=next.value+'\n';assert.ok(text.length<3*1024*1024);if(next.value==='')return text;}},close(){req.destroy();response.destroy();}};
      streams.push(stream);res(stream);
    });req.setTimeout(5000,()=>req.destroy(new Error('SSE fixture deadline')));req.on('error',rej);req.end();});
    const first=await subscribe(),second=await subscribe();
    for(const stream of [first,second])assert.match(await stream.frame(),/event: ready\ndata: .*"replay":false/);
    assert.equal((await request('/v1/collections/orders/documents',{method:'POST',headers:auth},'{"_id":"live","v":true}')).status,201);
    for(const stream of [first,second]){let frame;do{frame=await stream.frame();}while(frame.startsWith(':'));assert.match(frame,/event: change/);assert.ok(frame.includes('"id":"live"'));}
    assert.match(await first.frame(),/^:.*\n\n$/); // heartbeat reaches the client without proxy buffering
    first.close();second.close();
    await until(async()=>{const response=await request('/v1/status',{headers:auth});return response.status===200 && JSON.parse(response.text).connections.streaming===0;});
    // Browser adds a fixture-key exception; Node above independently verifies CA/TLS.
    const key=createPublicKey(await readFile(join(temp,'server.crt'))).export({type:'spki',format:'der'});const pin=createHash('sha256').update(key).digest('base64');
    browser=await chromium.launch({headless:true,args:[`--host-resolver-rules=MAP kiarni.test 127.0.0.1`,`--ignore-certificate-errors-spki-list=${pin}`,'--no-proxy-server']});
    const page=await browser.newPage();const browserErrors=[];page.on('pageerror',()=>browserErrors.push('page error'));await page.goto(origin);assert.equal(await page.locator('#session').isVisible(),false);
    await page.locator('#token').fill(token);await page.locator('#connect button').click();await page.locator('#session').waitFor();await page.getByRole('button',{name:'orders',exact:true}).click();await page.locator('#query-status').filter({hasText:'Consulta concluída'}).waitFor();assert.ok((await page.locator('#query-result').textContent()).includes('9007199254740993'));
    await page.locator('#refresh-metrics').click();await page.locator('#metrics-status').filter({hasText:'Coleta concluída'}).waitFor();await page.locator('#start-feed').click();await page.locator('#feed-status').filter({hasText:'Conectado.'}).waitFor();
    assert.equal((await request('/v1/collections/orders/documents',{method:'POST',headers:auth},'{"_id":"browser_live"}')).status,201);await page.locator('#feed-result').filter({hasText:'browser_live'}).waitFor();await page.locator('#disconnect').click();assert.equal(await page.locator('#session').isVisible(),false);
    assert.ok(await page.evaluate(()=>localStorage.length===0 && sessionStorage.length===0 && document.cookie===''));assert.equal(browserErrors.length,0);await browser.close();browser=null;
    const slow=await subscribe(true);
    const writingStarted=Date.now();
    for(let i=0;i<8;i++){
      const body=`{"_id":"slow_${i}","payload":"${'x'.repeat(500000)}"}`;
      assert.equal((await request('/v1/collections/orders/documents',{method:'POST',headers:auth},body)).status,201);
    }
    assert.ok(Date.now()-writingStarted<5000,'writes complete while the HTTPS subscriber does not consume events');slow.close();
    await until(async()=>{const response=await request('/v1/status',{headers:auth});return response.status===200 && JSON.parse(response.text).connections.streaming===0;});
    const requestId=await request('/v1/status',{headers:{...auth,'X-Request-ID':'private_request_marker'}});assert.match(requestId.headers['x-request-id'],/^req_[a-f0-9]{32}$/);
    assert.equal((await request('/v1/status',{headers:{...auth,Origin:[origin,origin]}})).status,403);
    assert.ok([400,401].includes((await request('/v1/status',{headers:{Authorization:[`Bearer ${token}`,`Bearer ${token}`]}})).status));
    assert.equal((await request('/v1/status?sensitive_query_marker',{headers:auth})).status,400);
    assert.equal((await request('/v1/collections/orders/documents?sensitive_query_marker',{method:'POST',headers:{...auth,'Content-Length':'1048577','Content-Type':'application/json'}})).status,413);
    const flood=await Promise.all(Array.from({length:80},()=>request('/v1/status',{headers:auth})));assert.ok(flood.some(response=>response.status===429),'proxy applies bounded connection/rate policy');
    await stop(nginx);await stop(backend);
    const access=await readFile(join(temp,'access.log'),'utf8');assert.ok(access.length>0);for(const line of access.trim().split('\n')){const record=JSON.parse(line);assert.deepEqual(Object.keys(record).sort(),['duration','request_id','status']);assert.ok(Number.isInteger(record.status));assert.match(record.request_id,/^(?:req_[a-f0-9]{32}|-|)$/);assert.match(record.duration,/^\d+\.\d+$/);}
    for(const text of [access,backendLog(),nginxLog(),await readFile(join(temp,'startup.log'),'utf8')])for(const secret of [token,marker,'sensitive_query_marker','private_request_marker'])assert.ok(!text.includes(secret),'no sensitive request content in telemetry');
    const version=await run(nginxBin,['-v']);console.log(`Proxy validation: ${version.stderr.trim()}; ${await run('openssl',['version']).then(r=>r.stdout.trim())}`);
  } finally {
    for(const stream of streams)stream.close();await browser?.close();await stop(nginx);await stop(backend);await rm(temp,{recursive:true,force:true});
  }
});
