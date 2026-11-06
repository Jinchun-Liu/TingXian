//! Fun-ASR-Realtime duplex transport. One task owns exactly one capture source.
//! No implicit replay: a connection lost after audio starts requires a new turn.
use crate::{Index,Result};
use futures_util::{SinkExt,StreamExt};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::{collections::HashSet,time::Duration};
use tokio::{net::TcpStream,sync::mpsc,time::{timeout,Instant}};
use tokio_tungstenite::{MaybeTlsStream,WebSocketStream,tungstenite::{Message,client::IntoClientRequest}};
use tokio_util::sync::CancellationToken;

#[derive(Clone,Debug,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum AsrMode {#[default] Local,Aliyun}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudAsrConfig {pub mode:AsrMode,pub workspace_id:String,pub silence_ms:u32}
impl Default for CloudAsrConfig {fn default()->Self{Self{mode:AsrMode::Local,workspace_id:String::new(),silence_ms:1300}}}
impl CloudAsrConfig {
    pub fn validate(&self)->Result<()> {
        let id=&self.workspace_id;
        if id.len()>80 || (!id.is_empty()&&!id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-')) || crate::contains_credential(id) {return Err("请填写百炼 Workspace ID，不要填写 API Key 或网址".into());}
        if self.mode==AsrMode::Aliyun && id.is_empty(){return Err("云转写需要百炼北京区 Workspace ID".into());}
        if !(200..=6000).contains(&self.silence_ms){return Err("断句静音须为200–6000毫秒".into());}Ok(())
    }
    pub fn endpoint(&self)->Result<String>{self.validate()?;if self.workspace_id.is_empty(){return Err("缺少 Workspace ID".into());}Ok(format!("wss://{}.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference",self.workspace_id))}
}
impl Index {
    pub async fn cloud_asr_config(&self)->Result<CloudAsrConfig>{
        let raw=self.setting("cloud_asr").await?;let config:CloudAsrConfig=if raw.is_empty(){CloudAsrConfig::default()}else{serde_json::from_str(&raw).map_err(|_|"云转写配置无效")?};config.validate()?;Ok(config)
    }
    pub async fn set_cloud_asr_config(&self,config:&CloudAsrConfig)->Result<()>{config.validate()?;self.set("cloud_asr",&serde_json::to_string(config).map_err(|e|e.to_string())?).await}
}

#[derive(Clone,Debug,PartialEq)]
pub enum AsrEvent {
    Ready,
    Sentence {id:u64,text:String,final_result:bool,begin_ms:u64,end_ms:Option<u64>,billed_seconds:Option<f64>},
    Finished,
}
pub struct TaskProtocol {id:String,started:bool,finished:bool,finals:HashSet<u64>}
impl TaskProtocol {
    pub fn new(id:String)->Result<Self>{
        if id.len()!=36 || id.bytes().enumerate().any(|(i,b)|if [8,13,18,23].contains(&i){b!=b'-'}else{!b.is_ascii_hexdigit()}){return Err("语音任务 ID 无效".into());}
        Ok(Self{id,started:false,finished:false,finals:HashSet::new()})
    }
    pub fn run(&self,config:&CloudAsrConfig,sample_rate:u32,language:&str,terms:&str)->Result<Value>{
        config.validate()?;if ![16000,48000].contains(&sample_rate){return Err("实时音频须为16或48kHz单声道PCM".into());}
        let mut parameters=json!({"format":"pcm","sample_rate":sample_rate,"max_sentence_silence":config.silence_ms,"heartbeat":true});
        if language!="auto" && !language.is_empty(){if !["zh","en"].contains(&language){return Err("云转写语种请选择自动、中文或英文".into());}parameters["language_hints"]=json!([language]);}
        // Only explicit domain terms go to the ASR, never unspoken AI suggestions.
        let terms=crate::redact_credentials(terms,None).chars().take(400).collect::<String>();
        let input=if terms.trim().is_empty(){json!({})}else{json!({"context":[{"role":"user","content":[{"type":"input_text","text":terms}]}]})};
        Ok(json!({"header":{"action":"run-task","task_id":self.id,"streaming":"duplex"},"payload":{"task_group":"audio","task":"asr","function":"recognition","model":"fun-asr-realtime","parameters":parameters,"input":input}}))
    }
    pub fn finish(&self)->Value{json!({"header":{"action":"finish-task","task_id":self.id,"streaming":"duplex"},"payload":{"input":{}}})}
    pub fn receive(&mut self,raw:&str)->Result<Option<AsrEvent>>{
        if raw.len()>1024*1024{return Err("云转写返回超过1MiB，已停止该通道".into());}
        let v:Value=serde_json::from_str(raw).map_err(|_|"云转写返回无效 JSON")?;
        if v["header"]["task_id"].as_str()!=Some(&self.id) || self.finished{return Ok(None);}
        match v["header"]["event"].as_str(){
            Some("task-started")=>{if self.started{return Ok(None);}self.started=true;Ok(Some(AsrEvent::Ready))},
            Some("task-failed")=>Err(format!("云转写任务失败：{}",v["header"]["error_code"].as_str().unwrap_or("unknown").chars().filter(|c|c.is_ascii_alphanumeric()||*c=='_').take(64).collect::<String>())),
            Some("task-finished")=>{if !self.started{return Err("云转写任务未就绪即结束".into());}self.finished=true;Ok(Some(AsrEvent::Finished))},
            Some("result-generated")=>{
                if !self.started{return Err("云转写任务尚未就绪".into());}
                let s=&v["payload"]["output"]["sentence"];
                if s["heartbeat"]==true{return Ok(None);}
                let id=s["sentence_id"].as_u64().filter(|n|*n>0).ok_or("云转写句子标识无效")?;
                if self.finals.contains(&id){return Ok(None);}
                let final_result=s["sentence_end"].as_bool().ok_or("云转写完成标记缺失")?;
                let text=s["text"].as_str().filter(|t|t.len()<=256*1024).ok_or("云转写文本无效")?.to_owned();
                let begin_ms=s["begin_time"].as_u64().ok_or("云转写时间无效")?;
                let end_ms=s["end_time"].as_u64();
                if (final_result&&end_ms.is_none())||end_ms.is_some_and(|e|e<begin_ms){return Err("云转写终点时间无效".into());}
                if final_result{self.finals.insert(id);}
                let billed_seconds=v["payload"]["usage"]["duration"].as_f64().filter(|n|n.is_finite()&&*n>=0.0);
                Ok(Some(AsrEvent::Sentence{id,text,final_result,begin_ms,end_ms,billed_seconds}))
            },
            _=>Err("云转写事件类型无效".into()),
        }
    }
}

type Socket=WebSocketStream<MaybeTlsStream<TcpStream>>;
pub struct PreparedTask {socket:Socket,protocol:TaskProtocol}
impl PreparedTask {
    /// Connect and wait for task-started before admitting any audio.
    pub async fn connect(config:&CloudAsrConfig,key:&str,id:String,sample_rate:u32,language:&str,terms:&str,cancel:&CancellationToken)->Result<Self>{
        Self::connect_to(&config.endpoint()?,config,key,id,sample_rate,language,terms,cancel).await
    }
    async fn connect_to(endpoint:&str,config:&CloudAsrConfig,key:&str,id:String,sample_rate:u32,language:&str,terms:&str,cancel:&CancellationToken)->Result<Self>{
        if key.trim().is_empty()||key.len()>4096||key.chars().any(char::is_control){return Err("云转写 API Key 缺失或格式无效".into());}
        let mut protocol=TaskProtocol::new(id)?;
        let run=protocol.run(config,sample_rate,language,terms)?;
        let mut request=endpoint.into_client_request().map_err(|_|"云转写地址无效")?;
        request.headers_mut().insert("Authorization",format!("Bearer {key}").parse().map_err(|_|"云转写凭证无效")?);
        request.headers_mut().insert("user-agent","TingXian/CloudASR".parse().unwrap());
        let work=async{
            let limits=tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(1024*1024)).max_frame_size(Some(1024*1024));
            let (mut socket,_)=tokio_tungstenite::connect_async_with_config(request,Some(limits),true).await.map_err(|e|match e{
                tokio_tungstenite::tungstenite::Error::Http(r)=>format!("云转写连接失败 HTTP {}",r.status().as_u16()),
                _=>"云转写连接失败，请检查网络与北京区配置".into(),
            })?;
            socket.send(Message::Text(run.to_string().into())).await.map_err(|_|"云转写启动发送失败")?;
            loop{match read_event(&mut socket,&mut protocol).await?{Some(AsrEvent::Ready)=>break,Some(AsrEvent::Finished)=>return Err("云转写提前结束".into()),_=>{}}}
            Ok(Self{socket,protocol})
        };
        tokio::select!{biased;_=cancel.cancelled()=>Err("云转写启动已取消".into()),result=timeout(Duration::from_secs(8),work)=>result.map_err(|_|"云转写连接或启动超时")?}
    }
    /// Bounded audio admission is owned by the caller. Closing input requests a
    /// final result; cancellation drops the socket without retrying or replaying.
    pub async fn stream<F,T>(mut self,mut audio:mpsc::Receiver<T>,cancel:CancellationToken,mut notify:F)->Result<()>
    where F:FnMut(AsrEvent)+Send,T:AsRef<[u8]>+Send {
        notify(AsrEvent::Ready);
        let mut finishing=false;let mut deadline=Instant::now()+Duration::from_secs(45);
        loop{
            tokio::select!{biased;
                _=cancel.cancelled()=>return Err("云转写已取消".into()),
                _=tokio::time::sleep_until(deadline)=>return Err(if finishing{"云转写结束超时，已保留收到的文字"}else{"云转写连接无响应，已暂停该通道"}.into()),
                event=read_event(&mut self.socket,&mut self.protocol)=>{
                    let event=event?;if !finishing{deadline=Instant::now()+Duration::from_secs(45);}
                    if let Some(event)=event{let done=matches!(event,AsrEvent::Finished);notify(event);if done{return if finishing{Ok(())}else{Err("云转写意外结束，当前发言可能不完整".into())};}}
                },
                pcm=audio.recv(),if !finishing=>{
                    let (message,_lease)=if let Some(pcm)=pcm{
                        let bytes=pcm.as_ref();if bytes.is_empty(){continue;}
                        if bytes.len()>192_000 || bytes.len()%2!=0{return Err("实时音频帧过大或不是PCM16".into());}
                        (Message::Binary(bytes.to_vec().into()),Some(pcm))
                    }else{finishing=true;deadline=Instant::now()+Duration::from_secs(8);(Message::Text(self.protocol.finish().to_string().into()),None)};
                    tokio::select!{biased;_=cancel.cancelled()=>return Err("云转写已取消".into()),sent=timeout(Duration::from_secs(3),self.socket.send(message))=>sent.map_err(|_|"云转写发送超时")?.map_err(|_|"云转写连接中断，未重发音频")?}
                }
            }
        }
    }
}
async fn read_event(socket:&mut Socket,protocol:&mut TaskProtocol)->Result<Option<AsrEvent>>{
    match socket.next().await {
        Some(Ok(Message::Text(text)))=>protocol.receive(&text),
        Some(Ok(Message::Ping(_)|Message::Pong(_)))=>Ok(None),
        _=>Err("云转写连接中断，未重发音频".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID:&str="01234567-0123-4567-89ab-0123456789ab";
    fn event(kind:&str)->Value{json!({"header":{"task_id":ID,"event":kind}})}
    #[test]
    fn config_and_protocol_preserve_partial_final_task_and_term_boundaries(){
        CloudAsrConfig::default().validate().unwrap();
        let config=CloudAsrConfig{mode:AsrMode::Aliyun,workspace_id:"llm-test".into(),..Default::default()};
        assert_eq!(config.endpoint().unwrap(),"wss://llm-test.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference");
        for workspace_id in ["","evil.test/path","user@host","ws?id=x"]{assert!(CloudAsrConfig{workspace_id:workspace_id.into(),..config.clone()}.validate().is_err());}
        let mut task=TaskProtocol::new(ID.into()).unwrap();
        let run=task.run(&config,48000,"auto",&"契约测试 null ".repeat(100)).unwrap();
        assert_eq!(run["payload"]["input"]["context"][0]["content"][0]["text"].as_str().unwrap().chars().count(),400);
        let mut sentence=event("result-generated");sentence["payload"]=json!({"output":{"sentence":{"sentence_id":1,"sentence_end":false,"text":"契约测试","begin_time":0,"end_time":null}},"usage":null});
        assert!(task.receive(&sentence.to_string()).is_err());
        assert_eq!(task.receive(&event("task-started").to_string()).unwrap(),Some(AsrEvent::Ready));
        assert!(matches!(task.receive(&sentence.to_string()).unwrap(),Some(AsrEvent::Sentence{final_result:false,..})));
        sentence["payload"]["output"]["sentence"]["sentence_end"]=true.into();sentence["payload"]["output"]["sentence"]["end_time"]=1000.into();
        assert!(matches!(task.receive(&sentence.to_string()).unwrap(),Some(AsrEvent::Sentence{final_result:true,..})));
        assert_eq!(task.receive(&sentence.to_string()).unwrap(),None,"duplicate final accepted");
        sentence["header"]["task_id"]="previous-task".into();assert_eq!(task.receive(&sentence.to_string()).unwrap(),None);
        assert_eq!(task.receive(&event("task-finished").to_string()).unwrap(),Some(AsrEvent::Finished));
        assert_eq!(task.receive(&event("task-started").to_string()).unwrap(),None,"finished task reopened");
    }
    #[tokio::test]
    async fn duplex_stream_waits_for_ready_drains_final_and_never_replays_after_drop(){
        for broken in [false,true]{
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint=format!("ws://{}",listener.local_addr().unwrap());
            let server=tokio::spawn(async move{
                let (tcp,_)=listener.accept().await.unwrap();let mut ws=tokio_tungstenite::accept_async(tcp).await.unwrap();
                let run:Value=serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();assert_eq!(run["header"]["action"],"run-task");
                assert!(timeout(Duration::from_millis(20),ws.next()).await.is_err(),"audio arrived before ready");
                ws.send(Message::Text(event("task-started").to_string().into())).await.unwrap();
                assert_eq!(ws.next().await.unwrap().unwrap(),Message::Binary(vec![1,0,2,0].into()));
                let mut result=event("result-generated");result["payload"]=json!({"output":{"sentence":{"sentence_id":1,"sentence_end":false,"text":"null 和零","begin_time":0,"end_time":null}}});
                ws.send(Message::Text(result.to_string().into())).await.unwrap();
                if broken{ws.close(None).await.unwrap();return;}
                let finish:Value=serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();assert_eq!(finish["header"]["action"],"finish-task");assert_eq!(finish["header"]["task_id"],ID);
                result["payload"]["output"]["sentence"]["sentence_end"]=true.into();result["payload"]["output"]["sentence"]["end_time"]=800.into();result["payload"]["usage"]=json!({"duration":1});
                ws.send(Message::Text(result.to_string().into())).await.unwrap();
                ws.send(Message::Text(result.to_string().into())).await.unwrap(); // Duplicate must not trigger twice.
                ws.send(Message::Text(event("task-finished").to_string().into())).await.unwrap();
            });
            let cancel=CancellationToken::new();
            let prepared=PreparedTask::connect_to(&endpoint,&CloudAsrConfig::default(),"synthetic",ID.into(),48000,"zh","StockMarketAgent",&cancel).await.unwrap();
            let (tx,rx)=mpsc::channel(2);tx.send(vec![1,0,2,0]).await.unwrap();drop(tx);
            let mut observed=vec![];let result=prepared.stream(rx,cancel,|event|observed.push(event)).await;
            assert_eq!(result.is_err(),broken);assert!(matches!(observed[0],AsrEvent::Ready));
            assert!(observed.iter().any(|e|matches!(e,AsrEvent::Sentence{final_result:false,text,..} if text=="null 和零")));
            assert_eq!(observed.iter().filter(|e|matches!(e,AsrEvent::Sentence{final_result:true,..})).count(),if broken{0}else{1});
            assert_eq!(observed.contains(&AsrEvent::Finished),!broken);server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn cancellation_interrupts_start_and_active_stream_without_reconnect(){
        for started in [false,true]{
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=format!("ws://{}",listener.local_addr().unwrap());
            let cancel=CancellationToken::new();let stop=cancel.clone();
            let server=tokio::spawn(async move{
                let (tcp,_)=listener.accept().await.unwrap();let mut ws=tokio_tungstenite::accept_async(tcp).await.unwrap();ws.next().await.unwrap().unwrap();
                if started{ws.send(Message::Text(event("task-started").to_string().into())).await.unwrap();}
                tokio::time::sleep(Duration::from_millis(20)).await;stop.cancel();
                assert!(timeout(Duration::from_millis(80),listener.accept()).await.is_err(),"cancel caused reconnection");
            });
            let prepared=PreparedTask::connect_to(&endpoint,&CloudAsrConfig::default(),"synthetic",ID.into(),48000,"auto","",&cancel).await;
            if started{let (_tx,rx)=mpsc::channel::<Vec<u8>>(1);assert!(timeout(Duration::from_secs(1),prepared.unwrap().stream(rx,cancel,|_|{})).await.unwrap().unwrap_err().contains("取消"));}
            else{assert!(prepared.err().unwrap().contains("取消"));}
            server.await.unwrap();
        }
    }
}
