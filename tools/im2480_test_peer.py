"""Independent Windows COM test peer. Only explicitly named com0com endpoints.

No emulator implementation or runtime files are imported from another project.
"""
from __future__ import annotations
import ctypes as C
from ctypes import wintypes as W
import struct
import threading
import time

K=C.WinDLL('kernel32',use_last_error=True)
K.CreateFileW.argtypes=[W.LPCWSTR,W.DWORD,W.DWORD,C.c_void_p,W.DWORD,W.DWORD,W.HANDLE];K.CreateFileW.restype=W.HANDLE
K.CloseHandle.argtypes=[W.HANDLE]
K.BuildCommDCBW.argtypes=[W.LPCWSTR,C.c_void_p]
K.GetCommState.argtypes=[W.HANDLE,C.c_void_p];K.SetCommState.argtypes=[W.HANDLE,C.c_void_p]
K.SetCommTimeouts.argtypes=[W.HANDLE,C.c_void_p]
K.ReadFile.argtypes=[W.HANDLE,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),C.c_void_p]
K.WriteFile.argtypes=[W.HANDLE,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),C.c_void_p]
K.QueryDosDeviceW.argtypes=[W.LPCWSTR,W.LPWSTR,W.DWORD]

def crc(payload):
    value=0xffff
    for byte in payload:
        value^=byte
        for _ in range(8):value=(value>>1)^0xa001 if value&1 else value>>1
    return struct.pack('<H',value)

class Com:
    def __init__(self,port,baud=9600,require_virtual=True):
        assert port.startswith('COM') and port[3:].isdigit() and int(port[3:])>0
        mapped=C.create_unicode_buffer(2048)
        assert K.QueryDosDeviceW(port,mapped,2048),C.WinError(C.get_last_error())
        if require_virtual:assert 'com0com' in mapped.value.lower(), 'Only a dedicated com0com pair is accepted'
        self.handle=K.CreateFileW('\\\\.\\'+port,0xc0000000,0,None,3,0,None)
        if self.handle==C.c_void_p(-1).value:raise C.WinError(C.get_last_error())
        try:
            dcb=C.create_string_buffer(28);struct.pack_into('<I',dcb,0,28)
            assert K.GetCommState(self.handle,dcb)
            assert K.BuildCommDCBW(f'baud={baud} parity=n data=8 stop=1 dtr=off rts=off',dcb)
            assert K.SetCommState(self.handle,dcb)
            timeouts=(W.DWORD*5)(0xffffffff,0xffffffff,25,0,100)
            assert K.SetCommTimeouts(self.handle,timeouts)
        except BaseException:self.close();raise
    def read(self,count=256):
        buffer=C.create_string_buffer(count);done=W.DWORD()
        if not K.ReadFile(self.handle,buffer,count,C.byref(done),None):raise C.WinError(C.get_last_error())
        return buffer.raw[:done.value]
    def write(self,data):
        done=W.DWORD();buffer=C.create_string_buffer(data)
        if not K.WriteFile(self.handle,buffer,len(data),C.byref(done),None):raise C.WinError(C.get_last_error())
        assert done.value==len(data)
    def close(self):
        if self.handle:K.CloseHandle(self.handle);self.handle=None

def measurement(address):
    # Fresh test values, with deliberately negative signed counts and exact fractions.
    words=[0]*31;words[1:4]=[address*10+1,address*10+2,address*10+3]
    signed=(-100000-address*17)&0xffffffff;words[16]=signed&65535;words[17]=signed>>16
    for offset,value in [(18,address+.25),(20,address+.5)]:words[offset:offset+2]=struct.unpack('>HH',struct.pack('<f',value))
    return words

class Peer:
    def __init__(self,port,baud=9600):
        self.com=Com(port,baud);self.stop=threading.Event();self.lock=threading.Lock();self.modes={};self.requests=[];self.error=None
        self.thread=threading.Thread(target=self.serve,daemon=True);self.thread.start()
    def mode(self,address,mode):
        with self.lock:self.modes[address]=mode
    def serve(self):
        buffer=b''
        try:
            while not self.stop.is_set():
                buffer=(buffer+self.com.read())[-1024:]
                while len(buffer)>=8:
                    frame=buffer[:8]
                    if crc(frame[:6])!=frame[6:]:buffer=buffer[1:];continue
                    buffer=buffer[8:]
                    address,function,start,count=struct.unpack('>BBHH',frame[:6])
                    with self.lock:
                        mode=self.modes.get(address,'good');self.requests.append((address,function,start,count))
                    assert function==3, 'The acquisition plugin must never send write commands'
                    if mode=='silent':continue
                    if start==0 and count==10:
                        words=[0x494d,0x2480,0,0,0,96,address,0,0,1 if mode=='order' else 0]
                    elif start==0x400 and count==31:
                        words=measurement(address)
                        if mode=='nan':words[18:20]=struct.unpack('>HH',struct.pack('<f',float('nan')))
                    else:raise AssertionError((start,count))
                    if mode=='exception':payload=bytes([address,0x83,2])
                    else:payload=bytes([address+1 if mode=='address' else address,3,count*2])+struct.pack('>'+'H'*count,*words)
                    response=payload+crc(payload)
                    if mode=='crc':response=response[:-1]+bytes([response[-1]^0x80])
                    if mode=='short':response=response[:4]
                    if mode=='fragment':
                        for offset in range(0,len(response),3):
                            self.com.write(response[offset:offset+3]);time.sleep(.001)
                    else:self.com.write(response)
        except BaseException as e:self.error=repr(e)
    def close(self):
        self.stop.set();self.thread.join(timeout=2);assert not self.thread.is_alive();self.com.close()
        assert self.error is None,self.error
