// AnPod AOA Phase-2 proof-of-concept (Windows PowerShell Add-Type / .NET Framework)
// Supported target before switch: iQOO 11 V2243A, VID_2D95 PID_6013 MI_02.
// Accessory after switch: only Google VID_18D1, PID_2D00 or PID_2D01 MI_00.
// DO NOT use this library with untrusted hardware. Source-only experimental probe.
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class AnPodAoaPhase2
{
    const uint PRESENT = 0x02;
    const uint INTERFACE = 0x10;
    const uint ALLCLASSES = 0x04;
    const uint READ = 0x80000000;
    const uint WRITE = 0x40000000;
    const uint SHARE_READ = 1;
    const uint SHARE_WRITE = 2;
    const uint OPEN_EXISTING = 3;
    const uint OVERLAPPED = 0x40000000;
    const uint PIPE_TRANSFER_TIMEOUT = 3;
    const int ERROR_NO_MORE_ITEMS = 259;
    const int BLOCK = 4096;
    static readonly Guid AdbGuid = new Guid("F72FE0D4-CBCB-407D-8814-9ED673D0DD6B");

    [StructLayout(LayoutKind.Sequential)]
    struct SP_DEVICE_INTERFACE_DATA {
        public int cbSize;
        public Guid InterfaceClassGuid;
        public int Flags;
        public IntPtr Reserved;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct SP_DEVINFO_DATA {
        public int cbSize;
        public Guid ClassGuid;
        public uint DevInst;
        public IntPtr Reserved;
    }
    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    struct WINUSB_SETUP_PACKET {
        public byte RequestType;
        public byte Request;
        public ushort Value;
        public ushort Index;
        public ushort Length;
    }
    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    struct USB_INTERFACE_DESCRIPTOR {
        public byte bLength;
        public byte bDescriptorType;
        public byte bInterfaceNumber;
        public byte bAlternateSetting;
        public byte bNumEndpoints;
        public byte bInterfaceClass;
        public byte bInterfaceSubClass;
        public byte bInterfaceProtocol;
        public byte iInterface;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct WINUSB_PIPE_INFORMATION {
        public int PipeType;
        public byte PipeId;
        public ushort MaximumPacketSize;
        public byte Interval;
    }
    [DllImport("setupapi.dll", EntryPoint="SetupDiGetClassDevsW", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SetupDiGetClassDevs(ref Guid ClassGuid, string Enumerator, IntPtr hwnd, uint Flags);
    [DllImport("setupapi.dll", EntryPoint="SetupDiGetClassDevsW", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SetupDiGetClassDevsAll(IntPtr ClassGuid, string Enumerator, IntPtr hwnd, uint Flags);
    [DllImport("setupapi.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetupDiEnumDeviceInterfaces(IntPtr Set, IntPtr DevInfo, ref Guid Guid, uint Index, ref SP_DEVICE_INTERFACE_DATA Data);
    [DllImport("setupapi.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetupDiEnumDeviceInterfaces(IntPtr Set, ref SP_DEVINFO_DATA DevInfo, IntPtr Guid, uint Index, ref SP_DEVICE_INTERFACE_DATA Data);
    [DllImport("setupapi.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetupDiEnumDeviceInfo(IntPtr Set, uint Index, ref SP_DEVINFO_DATA Data);
    [DllImport("setupapi.dll", EntryPoint="SetupDiGetDeviceInstanceIdW", CharSet=CharSet.Unicode, SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetupDiGetDeviceInstanceId(IntPtr Set, ref SP_DEVINFO_DATA DevInfo, StringBuilder InstanceId, uint Size, out uint Required);
    [DllImport("setupapi.dll", EntryPoint="SetupDiGetDeviceInterfaceDetailW", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr Set, ref SP_DEVICE_INTERFACE_DATA Data, IntPtr Detail, uint Size, out uint Needed, IntPtr DevInfo);
    [DllImport("setupapi.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetupDiDestroyDeviceInfoList(IntPtr Set);
    [DllImport("kernel32.dll", EntryPoint="CreateFileW", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFile(string Name, uint Access, uint Share, IntPtr Security, uint Disposition, uint Flags, IntPtr Template);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_Initialize(SafeFileHandle File, out IntPtr Interface);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_Free(IntPtr Interface);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_SetPipePolicy(IntPtr Interface, byte PipeId, uint Type, uint Size, ref uint Value);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_ControlTransfer(IntPtr Interface, WINUSB_SETUP_PACKET Setup, [In,Out] byte[] Data,
        uint DataLength, out uint Transferred, IntPtr Overlapped);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_QueryInterfaceSettings(IntPtr Interface, byte Alternate, out USB_INTERFACE_DESCRIPTOR Desc);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_QueryPipe(IntPtr Interface, byte Alternate, byte PipeIndex, out WINUSB_PIPE_INFORMATION Pipe);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_ReadPipe(IntPtr Interface, byte Pipe, [Out] byte[] Data, uint Length, out uint Transferred, IntPtr Overlapped);
    [DllImport("winusb.dll", SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WinUsb_WritePipe(IntPtr Interface, byte Pipe, [In] byte[] Data, uint Length, out uint Transferred, IntPtr Overlapped);

    static string Error(string Op) { int n = Marshal.GetLastWin32Error(); return Op + " Win32=" + n + " (" + new Win32Exception(n).Message + ")"; }
    static void Check(bool ok, string what) { if (!ok) throw new InvalidOperationException(Error(what)); }
    static string DetailPath(IntPtr Set, ref SP_DEVICE_INTERFACE_DATA Data) {
        uint needed;
        SetupDiGetDeviceInterfaceDetail(Set, ref Data, IntPtr.Zero, 0, out needed, IntPtr.Zero);
        if (needed < 8 || needed > 16384) throw new InvalidOperationException("Unexpected device interface detail size " + needed);
        IntPtr p = Marshal.AllocHGlobal((int)needed);
        try {
            Marshal.WriteInt32(p, IntPtr.Size == 8 ? 8 : 6);
            uint got;
            Check(SetupDiGetDeviceInterfaceDetail(Set, ref Data, p, needed, out got, IntPtr.Zero), "SetupDiGetDeviceInterfaceDetail");
            return Marshal.PtrToStringUni(IntPtr.Add(p, 4));
        } finally { Marshal.FreeHGlobal(p); }
    }
    static List<string> AdbPaths() {
        Guid g=AdbGuid; var res=new List<string>();
        IntPtr h=SetupDiGetClassDevs(ref g,null,IntPtr.Zero,PRESENT|INTERFACE);
        if(h == new IntPtr(-1)) throw new InvalidOperationException(Error("SetupDiGetClassDevs ADB"));
        try { for(uint i=0;;i++) {
            var d=new SP_DEVICE_INTERFACE_DATA(); d.cbSize=Marshal.SizeOf(typeof(SP_DEVICE_INTERFACE_DATA));
            if(!SetupDiEnumDeviceInterfaces(h,IntPtr.Zero,ref g,i,ref d)) {
                if(Marshal.GetLastWin32Error()==ERROR_NO_MORE_ITEMS)break;
                throw new InvalidOperationException(Error("SetupDiEnumDeviceInterfaces ADB"));
            }
            string p=DetailPath(h,ref d); string s=p.ToLowerInvariant();
            if(s.Contains("vid_2d95") && s.Contains("pid_6013") && s.Contains("mi_02"))res.Add(p);
        }}finally{SetupDiDestroyDeviceInfoList(h);}return res;
    }
    static List<string> AccessoryPaths() {
        var res=new List<string>();
        IntPtr h=SetupDiGetClassDevsAll(IntPtr.Zero,null,IntPtr.Zero,PRESENT|ALLCLASSES|INTERFACE);
        if(h==new IntPtr(-1))throw new InvalidOperationException(Error("SetupDiGetClassDevs accessory"));
        try {for(uint i=0;;i++) {
            var dev=new SP_DEVINFO_DATA();dev.cbSize=Marshal.SizeOf(typeof(SP_DEVINFO_DATA));
            if(!SetupDiEnumDeviceInfo(h,i,ref dev)){
                if(Marshal.GetLastWin32Error()==ERROR_NO_MORE_ITEMS)break;
                throw new InvalidOperationException(Error("SetupDiEnumDeviceInfo"));
            }
            var idBuf=new StringBuilder(1024);uint required;
            if(!SetupDiGetDeviceInstanceId(h,ref dev,idBuf,(uint)idBuf.Capacity,out required))continue;
            string id=idBuf.ToString().ToLowerInvariant();
            if(!id.Contains("vid_18d1"))continue;
            bool pid0=id.Contains("pid_2d00"), pid1=id.Contains("pid_2d01");
            if(!(pid0 || pid1))continue;
            // On 2D01 composite devices MI_01 is ADB. Never select it for this probe.
            if(pid1 && !id.Contains("mi_00"))continue;
            for(uint k=0;;k++) {
                var iface=new SP_DEVICE_INTERFACE_DATA();iface.cbSize=Marshal.SizeOf(typeof(SP_DEVICE_INTERFACE_DATA));
                if(!SetupDiEnumDeviceInterfaces(h,ref dev,IntPtr.Zero,k,ref iface)){
                    if(Marshal.GetLastWin32Error()==ERROR_NO_MORE_ITEMS)break;
                    break;
                }
                string p=DetailPath(h,ref iface);
                if(!String.IsNullOrEmpty(p) && !res.Contains(p,StringComparer.OrdinalIgnoreCase))res.Add(p);
            }
        }}finally{SetupDiDestroyDeviceInfoList(h);}return res;
    }
    sealed class Device : IDisposable {
        public SafeFileHandle File;
        public IntPtr Usb;
        public Device(string path){
            File=CreateFile(path,READ|WRITE,SHARE_READ|SHARE_WRITE,IntPtr.Zero,OPEN_EXISTING,OVERLAPPED,IntPtr.Zero);
            if(File==null || File.IsInvalid)throw new InvalidOperationException(Error("CreateFile"));
            try{Check(WinUsb_Initialize(File,out Usb),"WinUsb_Initialize");}
            catch{File.Dispose();throw;}
        }
        public void Timeout(byte pipe,uint ms){uint m=ms;Check(WinUsb_SetPipePolicy(Usb,pipe,PIPE_TRANSFER_TIMEOUT,4,ref m),"WinUsb_SetPipePolicy");}
        public void Dispose(){if(Usb!=IntPtr.Zero){WinUsb_Free(Usb);Usb=IntPtr.Zero;}if(File!=null){File.Dispose();File=null;}}
        public byte[] Control(byte type,byte req,ushort index,byte[] data){
            ushort len=(ushort)(data==null?0:data.Length);
            var setup=new WINUSB_SETUP_PACKET{RequestType=type,Request=req,Value=0,Index=index,Length=len};
            uint n;Check(WinUsb_ControlTransfer(Usb,setup,data,(uint)len,out n,IntPtr.Zero),"ControlTransfer request " + req);
            if(n!=len)throw new InvalidOperationException("ControlTransfer request " + req + " transferred " + n + "/" + len);
            return data;
        }
    }
    static string OldPath(){var paths=AdbPaths();if(paths.Count!=1)throw new InvalidOperationException("Expected one iQOO 11 ADB interface; found " + paths.Count);return paths[0];}
    public static string Preflight(){
        using(var d=new Device(OldPath())){
            d.Timeout(0,3000);
            byte[] b=d.Control(0xC0,51,0,new byte[2]);
            int ver=b[0]|(b[1]<<8);if(ver<1)throw new InvalidOperationException("AOA version="+ver);
            return "SUCCESS AOA protocol="+ver+"; no USB switch performed.";
        }
    }
    static void StringId(Device d,ushort id,string text){
        byte[] raw=Encoding.UTF8.GetBytes(text+"\0");
        if(raw.Length>256)throw new InvalidOperationException("AOA accessory string too long");
        d.Control(0x40,52,id,raw);
    }
    public static string SwitchAccessory(){
        return SwitchAccessory(false);
    }
    public static string SwitchAnPodReceiver(){
        return SwitchAccessory(true);
    }
    static string SwitchAccessory(bool productIdentity){
        using(var d=new Device(OldPath())){
            d.Timeout(0,3000);
            byte[] b=d.Control(0xC0,51,0,new byte[2]);
            int ver=b[0]|(b[1]<<8);if(ver<1)throw new InvalidOperationException("AOA unavailable");
            StringId(d,0,productIdentity?"AnPod":"AnPod Lab");
            StringId(d,1,productIdentity?"AnPod USB Direct":"AOA Bulk Probe");
            StringId(d,2,productIdentity?"AnPod USB Direct audio receiver":"AnPod compatibility experiment - not a music player");
            StringId(d,3,"1.0");
            StringId(d,4,"https://github.com/sanren1/AnPod");
            StringId(d,5,productIdentity?"ANPOD-AOA-USB-DIRECT":"ANPOD-AOA-PROBE");
            string result="All AOA identity strings were sent. ";
            try {d.Control(0x40,53,0,new byte[0]);result+="START_ACCESSORY (53) returned successfully.";}
            catch(Exception e){result+="START_ACCESSORY may have disconnected the old interface: "+e.Message;}
            return result+(productIdentity
                ? " Product identity requested: AnPod / AnPod USB Direct. Check whether VID_18D1 PID_2D00/2D01 enumerates."
                : " Check whether VID_18D1 PID_2D00/2D01 enumerates.");
        }
    }
    public static string InspectAccessory(){
        List<string> paths=AccessoryPaths();
        if(paths.Count==0)return "No accessible WinUSB device-interface path on VID_18D1:2D00/2D01 accessory data interface. Check PnP device, driver and permissions. Never replace ADB MI_01.";
        var report=new StringBuilder();report.AppendLine("Candidate AOA WinUSB paths: "+paths.Count);
        for(int k=0;k<paths.Count;k++){
            try{using(var d=new Device(paths[k])){
                USB_INTERFACE_DESCRIPTOR descriptor;
                Check(WinUsb_QueryInterfaceSettings(d.Usb,0,out descriptor),"WinUsb_QueryInterfaceSettings");
                report.AppendLine("path "+(k+1)+" interface "+descriptor.bInterfaceNumber+" endpoints "+descriptor.bNumEndpoints);
                for(byte j=0;j<descriptor.bNumEndpoints;j++){
                    WINUSB_PIPE_INFORMATION p;
                    Check(WinUsb_QueryPipe(d.Usb,0,j,out p),"WinUsb_QueryPipe");
                    report.AppendLine("  "+(p.PipeType==2?"Bulk":"Other")+" 0x"+p.PipeId.ToString("X2")+" MPS="+p.MaximumPacketSize);
                }
            }}catch(Exception e){report.AppendLine("candidate " +(k+1)+": "+e.Message);}
        }
        return report.ToString();
    }
    static void FillBlock(byte[] block, int seq){
        block[0]=(byte)'A';block[1]=(byte)'P';block[2]=(byte)'A';block[3]=(byte)'O';
        block[4]=1;block[5]=1;block[6]=0;block[7]=0;
        block[8]=(byte)(seq);block[9]=(byte)(seq>>8);block[10]=(byte)(seq>>16);block[11]=(byte)(seq>>24);
        block[12]=0;block[13]=16;block[14]=0;block[15]=0; // total block length = 4096 (little-endian)
        for(int i=16;i<block.Length;i++)block[i]=(byte)((seq*131+i*17+7)&255);
    }
    static string RunOnDevice(Device d,int rounds){
        USB_INTERFACE_DESCRIPTOR desc;Check(WinUsb_QueryInterfaceSettings(d.Usb,0,out desc),"WinUsb_QueryInterfaceSettings");
        byte inId=0,outId=0;
        for(byte k=0;k<desc.bNumEndpoints;k++){
            WINUSB_PIPE_INFORMATION p;Check(WinUsb_QueryPipe(d.Usb,0,k,out p),"WinUsb_QueryPipe");
            if(p.PipeType!=2)continue;
            if((p.PipeId&0x80)!=0)inId=p.PipeId;else outId=p.PipeId;
        }
        if(inId==0 || outId==0)throw new InvalidOperationException("No matching bulk IN+OUT endpoint pair (must use accessory interface, not ADB)");
        d.Timeout(inId,3000);d.Timeout(outId,3000);
        byte[] send=new byte[BLOCK];byte[] recv=new byte[BLOCK];
        var rtts=new List<double>();var all=Stopwatch.StartNew();
        for(int seq=1;seq<=rounds;seq++){
            FillBlock(send,seq);
            var timer=Stopwatch.StartNew();uint n;
            Check(WinUsb_WritePipe(d.Usb,outId,send,BLOCK,out n,IntPtr.Zero),"WinUsb_WritePipe sequence="+seq);
            if(n!=BLOCK)throw new InvalidOperationException("Partial USB write ("+n+")");
            int collected=0;
            while(collected<BLOCK){
                int remaining=BLOCK-collected;
                byte[] fragment=new byte[remaining];
                Check(WinUsb_ReadPipe(d.Usb,inId,fragment,(uint)remaining,out n,IntPtr.Zero),"WinUsb_ReadPipe sequence="+seq);
                if(n==0 || n>remaining)throw new InvalidOperationException("Unexpected USB read size ("+n+")");
                Buffer.BlockCopy(fragment,0,recv,collected,(int)n);
                collected+=(int)n;
            }
            timer.Stop();
            for(int i=0;i<BLOCK;i++)if(send[i]!=recv[i])throw new InvalidOperationException("Payload mismatch at sequence="+seq+" index="+i);
            rtts.Add(timer.Elapsed.TotalMilliseconds);
        }
        all.Stop();rtts.Sort();
        int pos95=(int)Math.Ceiling(.95*rounds)-1;
        double bits=(double)rounds*BLOCK*2*8;
        return "SUCCESS: Bulk echo "+rounds+" x "+BLOCK+" bytes; 100% verified.\n"+
            "Round-trip application echo p50="+rtts[rounds/2].ToString("F2")+" ms, p95="+rtts[pos95].ToString("F2")+" ms.\n"+
            "Bidirectional echo goodput="+(bits/1000000.0/all.Elapsed.TotalSeconds).ToString("F2")+" Mbit/s (includes Android echo and per-block request/response; NOT unidirectional USB throughput or audio latency).";
    }
    public static string BulkEcho(int rounds){
        if(rounds<1 || rounds>512)throw new ArgumentOutOfRangeException("rounds","Allowed 1..512");
        var paths=AccessoryPaths();
        if(paths.Count==0)throw new InvalidOperationException("No WinUSB interface for AOA accessory data interface; inspect VID_18D1 PnP drivers first.");
        if(paths.Count>1)throw new InvalidOperationException("Multiple AOA WinUSB interfaces found; disconnect other AOA accessories before running Bulk.");
        var errors=new List<string>();
        foreach(string p in paths){try{using(var d=new Device(p)){return RunOnDevice(d,rounds);}}catch(Exception e){errors.Add(e.Message);}}
        throw new InvalidOperationException("No accessory Bulk transport succeeded: "+String.Join(" | ",errors.ToArray()));
    }
}
