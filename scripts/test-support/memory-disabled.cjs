// Strict default-off native contract for pre-existing page regressions.
module.exports=(name)=>{
 if(name==='interview_memory_open')return null;
 if(name==='interview_memory_status')return {active:false};
 if(name==='interview_memory_config_get')return {key_configured:false,config:{enabled:false,min_events:8,cooldown_ms:60000,max_requests:8,session_units:200000,profile:{id:'compaction',name:'会话记忆',enabled:true,endpoint:'https://synthetic.invalid/chat/completions',protocol:'openai',request:{model:'synthetic',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:60,budget:{context_tokens:262144,max_output_tokens:8192,safety_tokens:2048}}}}};
 throw Error('Unexpected memory IPC in default-off regression: '+name);
};
