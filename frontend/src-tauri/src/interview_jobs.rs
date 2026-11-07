//! Three independent questions, one job per configured answer profile.
use std::{collections::HashMap,sync::{Arc,Mutex}};
use tokio_util::sync::CancellationToken;

struct Job {revision:u32,profile:String,cancel:CancellationToken}
#[derive(Default)]
pub(crate) struct AnswerJobs {jobs:Mutex<HashMap<String,Job>>}
pub(crate) struct AnswerLease {owner:Arc<AnswerJobs>,id:String,revision:u32,pub cancel:CancellationToken,pub profile:String}
impl AnswerJobs {
    pub fn reserve(self:&Arc<Self>,id:&str,revision:u32,profiles:&[String])->Result<AnswerLease,String>{
        let mut jobs=self.jobs.lock().unwrap_or_else(|e|e.into_inner());
        if jobs.contains_key(id){return Err("该题仍在生成，请先等待或取消".into());}
        if jobs.len()>=3{return Err("三个回答通道正在使用".into());}
        let profile=profiles.iter().find(|p|!jobs.values().any(|j|&j.profile==*p)).ok_or("没有空闲且已配置的回答接口")?.clone();
        let cancel=CancellationToken::new();jobs.insert(id.into(),Job{revision,profile:profile.clone(),cancel:cancel.clone()});
        Ok(AnswerLease{owner:self.clone(),id:id.into(),revision,cancel,profile})
    }
    pub fn cancel_all(&self){for job in self.jobs.lock().unwrap_or_else(|e|e.into_inner()).values(){job.cancel.cancel();}}
    pub fn is_empty(&self)->bool{self.jobs.lock().unwrap_or_else(|e|e.into_inner()).is_empty()}
    pub fn len(&self)->usize{self.jobs.lock().unwrap_or_else(|e|e.into_inner()).len()}
}
impl Drop for AnswerLease {
    fn drop(&mut self){let mut jobs=self.owner.jobs.lock().unwrap_or_else(|e|e.into_inner());if jobs.get(&self.id).is_some_and(|j|j.revision==self.revision){jobs.remove(&self.id);}}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn three_questions_are_isolated_and_cancel_retains_slots_until_drained(){
        let jobs=Arc::new(AnswerJobs::default());let profiles=vec!["a".into(),"b".into(),"c".into()];
        let a=jobs.reserve("q1",1,&profiles).unwrap();let b=jobs.reserve("q2",2,&profiles).unwrap();let c=jobs.reserve("q3",3,&profiles).unwrap();
        assert_ne!(a.profile,b.profile);assert_ne!(b.profile,c.profile);assert!(jobs.reserve("q4",4,&profiles).is_err());assert!(jobs.reserve("q1",5,&profiles).is_err());
        drop(b);let b=jobs.reserve("q4",4,&profiles).unwrap();assert_eq!(b.profile,"b");assert!(!a.cancel.is_cancelled());
        jobs.cancel_all();assert!(a.cancel.is_cancelled()&&b.cancel.is_cancelled()&&c.cancel.is_cancelled());assert!(jobs.reserve("q5",5,&profiles).is_err());
        drop(a);drop(b);drop(c);assert!(jobs.is_empty());assert!(jobs.reserve("q5",5,&profiles).is_ok());
    }
}
