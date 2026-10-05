export async function api<T>(path:string, body?:unknown):Promise<T> {
 const response=await fetch('/api/'+path,body===undefined?{}:{method:'POST',headers:{'Content-Type':'application/json','X-Strata-Local':'1'},body:JSON.stringify(body)});
 const text=await response.text();
 let value:unknown;try{value=JSON.parse(text)}catch{throw new Error(text||`HTTP ${response.status}`)}
 if(!response.ok)throw new Error((value as {error?:string}).error||`HTTP ${response.status}`);
 return value as T;
}
export const number=(n:number|null|undefined, digits=1)=>n==null||!Number.isFinite(n)?'—':new Intl.NumberFormat('en',{maximumFractionDigits:digits}).format(n);
export function bytes(n:number|null|undefined):string {if(n==null)return '—';const sign=n<0?'-':'';const a=Math.abs(n);const i=a?Math.min(3,Math.floor(Math.log(a)/Math.log(1024))):0;return `${sign}${number(a/1024**i)} ${['B','KiB','MiB','GiB'][i]}`;}
export interface Sizes {records:number;logical_bytes:number;physical_bytes:number;compression_ratio:number|null;storage_saved_bytes:number;segments:number;blocks:number}
export interface Dataset {name:string;metrics?:Sizes;error?:string;schema?:{id:string}}
export interface Metrics {live:boolean;elapsed:number;records_processed:number;logical_input_bytes:number;physical_bytes_written:number;compression_ratio:number;records_per_sec:number;logical_mb_per_sec:number;physical_mb_per_sec:number;blocks_per_sec:number;worker_count:number;active_workers?:number;queue_depth_parse:number;queue_depth_encode:number;queue_depth_write:number;files_completed:number;files_seen:number;total_files?:number;current_input_file:string|null}
export interface Job {id:number;operation:string;dataset:string;status:string;error:string|null;result:unknown}
export interface Trial {steps:string[];encoded_bytes:number;logical_bytes:number;encode_mb_s:number;decode_mb_s:number;compression_ratio?:number;feasible:boolean}
export interface Decision {name:string;logical_type:string;reason:string;selected:Trial;trials:Trial[]}
export interface Optimization {scope:string;dataset_records:number;report:{sample_rows:number;columns:Decision[];profile_toml:string}}
export interface Analysis {scope:string;report:{record_count:number;columns:{name:string;logical_type:string;cardinality:number;row_count:number}[]}}
export interface Segment {ordinal:number;file:string;metrics:Sizes;record_start:number;record_end_exclusive:number;health:string;metadata:{blocks:{id:number;row_count:number;block_length:number;compression_codec:number}[]}}
export interface Detail {metrics:Sizes;manifest:{schema:{id:string;fields:{name:string;logical_type:unknown}[]}};segments:Segment[];columns:{name:string;logical_type:unknown;logical_bytes:number;semantic_encoded_bytes:number}[];indexes_present:boolean}
