const net = require('node:net');
const http = require('node:http');
const { spawn } = require('node:child_process');
const path = require('node:path');
const listen = s => new Promise(resolve => s.listen(0, '127.0.0.1', () => resolve(s.address().port)));
(async () => {
  const mock = net.createServer(socket => {
    let data = '';
    socket.on('data', chunk => {
      data += chunk;
      if (!data.includes('\r\n\r\n')) return;
      console.log('MOCK REQUEST:', data.split('\r\n')[0]);
      const connect = data.startsWith('CONNECT ');
      data = '';
      if (connect) socket.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      else socket.end('HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nACCOUNT_A');
    });
  });
  const upstream = await listen(mock);
  const portReservation = net.createServer(); const port = await listen(portReservation); await new Promise(r => portReservation.close(r));
  const child = spawn(path.resolve('src-tauri/proxy-core/xray.exe'), ['run', '-format', 'json', '-config', 'stdin:'], { windowsHide: true });
  child.stderr.on('data', d => process.stdout.write(d)); child.stdout.on('data', d => process.stdout.write(d));
  child.stdin.end(JSON.stringify({log:{loglevel:'debug'},inbounds:[{listen:'127.0.0.1',port,protocol:'http',settings:{}}],outbounds:[{protocol:'http',settings:{servers:[{address:'127.0.0.1',port:upstream}]}}]}));
  try {
    await new Promise(r => setTimeout(r, 700));
    const request = http.get({hostname:'127.0.0.1',port,path:'http://127.0.0.1:32099/test',headers:{Host:'127.0.0.1:32099'}}, res => {
      let body=''; res.on('data',d=>body+=d); res.on('end',()=>console.log('RESULT',res.statusCode,JSON.stringify(body)));
    });
    await new Promise((resolve,reject)=>{ request.on('close',resolve); request.on('error',reject); request.setTimeout(5000,()=>request.destroy(new Error('timeout'))); });
  } finally { child.kill(); mock.close(); }
})().catch(e=>{console.error(e);process.exit(1)});
