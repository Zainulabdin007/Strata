import {chromium} from '@playwright/test';
import {mkdtemp,cp,rm,mkdir,readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {spawn} from 'node:child_process';
import assert from 'node:assert/strict';
const repo=resolve('..'),root=await mkdtemp(join(tmpdir(),'strata-lab-test-'));
await cp(join(repo,'examples'),join(root,'examples'),{recursive:true});
await mkdir(join(root,'strata-lab'),{recursive:true});
await cp(resolve('dist'),join(root,'strata-lab/dist'),{recursive:true});
const server=spawn(join(repo,'target/debug/strata-api'),['--root',root,'--port','19161'],{stdio:'inherit'});
let browser;
try{
 for(let i=0;i<100;i++){try{const r=await fetch('http://127.0.0.1:19161/api/health');if(r.ok)break}catch{}await new Promise(r=>setTimeout(r,100));}
 browser=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
 const page=await browser.newPage({viewport:{width:1440,height:1000}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:19161/');
 await page.getByRole('button',{name:'Live ingest',exact:false}).click();
 await page.getByRole('button',{name:'START INGEST'}).click();
 await page.getByText('complete',{exact:true}).first().waitFor({timeout:30000});
 await page.getByRole('button',{name:'Overview',exact:false}).click();
 await page.getByText('Storage composition').waitFor();
 const d=await fetch('http://127.0.0.1:19161/api/datasets/market').then(r=>r.json());
 assert(d.metrics.records>0);assert(await page.locator('.metric').first().innerText().then(s=>s.includes(String(d.metrics.records))));
 await page.getByRole('button',{name:'Column analysis',exact:false}).click();
 await page.getByRole('button',{name:'PROFILE DATASET'}).click();
 await page.getByRole('button',{name:'PROFILE DATASET'}).waitFor({timeout:30000});
 await page.getByText(/records observed/).waitFor({timeout:30000});
 await page.getByRole('button',{name:'Pipelines',exact:false}).click();
 await page.locator('.pipeline').first().waitFor();
 await page.getByRole('button',{name:'Segments',exact:false}).click();
 await page.locator('summary').first().click();
 assert(await page.getByText('General codec').isVisible());
 await page.getByRole('button',{name:'Query',exact:false}).click();
 await page.getByRole('button',{name:'EXECUTE QUERY'}).click();
 await page.getByText('Execution statistics').waitFor();
 assert((await page.locator('pre').first().innerText()).includes('AAPL'));
 await page.getByRole('button',{name:'Benchmarks',exact:false}).click();
 assert.equal(await page.getByText('No measured runs available.').count(),6);
 // Display a report produced by the real release DBN -> Strata validation run.
 // No invented metric fixture: omit this optional integration if the corpus has not been prepared.
 const realReport=join(repo,'benchmarks/nasdaq/results/validation-first-validation-manual-w2.json');
 try{
  const measured=JSON.parse(await readFile(realReport,'utf8'));
  assert.equal(measured.records,measured.verified_records);
  await mkdir(join(root,'benchmarks/nasdaq/results'),{recursive:true});
  await cp(realReport,join(root,'benchmarks/nasdaq/results/measured.json'));
  await page.reload();
  await page.getByRole('button',{name:'Benchmarks',exact:false}).click();
  await page.getByLabel('Benchmark workload').waitFor();
  assert((await page.getByLabel('Benchmark workload').innerText()).includes('Databento'));
  assert.equal(await page.locator('.bar-row').count(),5);
  assert.equal(await page.getByText('No measured runs available.').count(),1); // scaling needs >1 worker count
  const reported=await fetch('http://127.0.0.1:19161/api/benchmarks').then(r=>r.json());
  assert.deepEqual(reported[0].canonical_metrics,measured.canonical_metrics);
  console.log('PASS real Nasdaq report: exact canonical metrics from API, corpus grouping, no fake worker scaling');
 }catch(e){if(e.code!=='ENOENT')throw e;console.log('Real corpus report not present; optional Nasdaq chart integration not run');}

 await page.getByRole('button',{name:'Experiment',exact:false}).click();
 await page.getByLabel('Block rows').fill('2');
 await page.getByLabel('Segment bytes').fill('1024');
 await page.getByRole('button',{name:'RUN ANALYSIS',exact:true}).click();
 await page.getByText('Measured candidates',{exact:true}).waitFor({timeout:120000});
 assert(await page.getByText('Plain + zstd',{exact:true}).isVisible());
 assert(await page.locator('tr.winner').count()===1);
 await page.getByRole('button',{name:'SAVE WINNER AS TOML'}).click();
 await page.getByText('Saved profiles/winner.toml',{exact:true}).waitFor();
 const job=await fetch('http://127.0.0.1:19161/api/jobs',{method:'POST',headers:{'Content-Type':'application/json','X-Strata-Local':'1'},body:JSON.stringify({operation:'ingest',dataset:'winner',input:'examples/data/market.csv',profile:'profiles/winner.toml'})}).then(r=>r.json());
 let checked;for(let i=0;i<100;i++){checked=await fetch(`http://127.0.0.1:19161/api/jobs/${job.id}`).then(r=>r.json());if(checked.status!=='running')break;await new Promise(r=>setTimeout(r,100));}
 assert.equal(checked.status,'complete',JSON.stringify(checked));
 const winnerData=await fetch('http://127.0.0.1:19161/api/datasets/winner').then(r=>r.json());
 assert.equal(winnerData.metrics.records,d.metrics.records);
 assert.equal(winnerData.manifest.block_row_count,2);
 console.log('PASS gate 20: real candidates, measured winner, saved TOML successfully re-ingested, block override applied');
 await page.setViewportSize({width:390,height:844});
 assert(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth));
 assert.deepEqual(errors,[]);
 console.log('PASS gate 19: real browser ingest, canonical overview, profiler, pipelines, segment decode metadata, query, honest empty charts, mobile width');
}finally{await browser?.close();server.kill();await new Promise(r=>server.once('exit',r));await rm(root,{recursive:true,force:true})}
