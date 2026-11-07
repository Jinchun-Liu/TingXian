"""Credential-free, durable reservations. Unknown usage keeps its full reserve."""
import json
import sqlite3
import threading
import time

class Stopped(RuntimeError): pass

class Ledger:
    def __init__(self,path,limit_units=0,max_calls=0):
        if type(limit_units) is not int or limit_units<0 or type(max_calls) is not int or max_calls<0: raise ValueError('Invalid limits')
        self.db=sqlite3.connect(path,check_same_thread=False,isolation_level=None)
        self.lock=threading.RLock()
        self.db.executescript('CREATE TABLE IF NOT EXISTS config(id INTEGER PRIMARY KEY, units INTEGER, calls INTEGER, cancelled INTEGER DEFAULT 0); CREATE TABLE IF NOT EXISTS attempts(id TEXT PRIMARY KEY, role TEXT, state TEXT, reserve INTEGER, actual INTEGER, usage TEXT, started REAL, ended REAL);')
        with self.lock:
            old=self.db.execute('SELECT units,calls FROM config WHERE id=1').fetchone()
            if old and old!=(limit_units,max_calls):
                self.db.close()
                raise ValueError('Budget changed; create a new experiment')
            self.db.execute('INSERT OR IGNORE INTO config(id,units,calls) VALUES(1,?,?)',(limit_units,max_calls))
    def cancelled(self): return bool(self.db.execute('SELECT cancelled FROM config WHERE id=1').fetchone()[0])
    def reserve(self,attempt,role,worst_units,input_units,output_units,window=262144,safety=2048):
        if any(type(n) is not int or n<0 for n in (worst_units,input_units,output_units,window,safety)): raise ValueError('Known nonnegative integer bounds required')
        if output_units<16 or safety<256 or input_units+output_units+safety>min(window,262144): raise ValueError('Request exceeds model/context limit')
        with self.lock:
            self.db.execute('BEGIN IMMEDIATE')
            try:
                cap,calls,stop=self.db.execute('SELECT units,calls,cancelled FROM config WHERE id=1').fetchone()
                if stop: raise Stopped('Experiment cancelled')
                if cap<=0 or calls<=0: raise Stopped('Paid execution disabled without a positive explicit budget')
                if worst_units<=0: raise ValueError('Worst request cost must be known and positive')
                spent,count=self.db.execute('SELECT COALESCE(SUM(COALESCE(actual,reserve)),0),COUNT(*) FROM attempts').fetchone()
                if spent+worst_units>cap or count>=calls: raise Stopped('Cumulative budget exhausted before sending')
                self.db.execute('INSERT INTO attempts(id,role,state,reserve,started) VALUES(?,?,?,?,?)',(attempt,role,'reserved',worst_units,time.time()))
                self.db.execute('COMMIT')
            except BaseException:
                self.db.execute('ROLLBACK');raise
    def finish(self,attempt,actual_units=None,usage=None):
        with self.lock:
            row=self.db.execute('SELECT state,reserve FROM attempts WHERE id=?',(attempt,)).fetchone()
            if not row or row[0]!='reserved': raise ValueError('Unknown or already completed attempt')
            if actual_units is not None and (type(actual_units) is not int or not 0<=actual_units<=row[1]):
                self.cancel();raise ValueError('Metering exceeded the reserved bound')
            state='cancelled' if self.cancelled() else ('unknown' if actual_units is None else 'completed')
            self.db.execute('UPDATE attempts SET state=?,actual=?,usage=?,ended=? WHERE id=?',(state,actual_units,json.dumps(usage) if usage is not None else None,time.time(),attempt))
            return state=='completed'
    def cancel(self):
        with self.lock: self.db.execute('UPDATE config SET cancelled=1 WHERE id=1')
    def close(self): self.db.close()
