use serde_json::{json, Value};
use std::{
    mem::{size_of, size_of_val},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Instant,
};
use windows::{
    core::{Error, Result, PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        NetworkManagement::IpHelper::*,
        Networking::WinSock::{AF_INET, AF_INET6},
        Security::{WinTrust::*, *},
        Storage::FileSystem::*,
        System::{Diagnostics::ToolHelp::*, RemoteDesktop::ProcessIdToSessionId, Threading::*},
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

unsafe fn token_user(process: HANDLE) -> Result<Vec<usize>> {
    let mut raw = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut raw)?;
    let token = Handle(raw);
    let mut bytes = 0;
    let err = GetTokenInformation(token.0, TokenUser, None, 0, &mut bytes).unwrap_err();
    if err.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() {
        return Err(err);
    }
    let mut data = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
    GetTokenInformation(
        token.0,
        TokenUser,
        Some(data.as_mut_ptr().cast()),
        bytes,
        &mut bytes,
    )?;
    Ok(data)
}

unsafe fn session(pid: u32) -> Result<u32> {
    let mut id = 0;
    ProcessIdToSessionId(pid, &mut id)?;
    Ok(id)
}

fn listener(ip: IpAddr, port: u32, scope: u32) -> Value {
    json!({"addr":ip.to_string(),"port":u16::from_be(port as u16),"scope_id":scope,
      "is_strict_loopback":ip==Ipv4Addr::LOCALHOST || ip==Ipv6Addr::LOCALHOST,
      "is_wildcard":ip.is_unspecified()})
}

unsafe fn listeners(pid: u32) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for family in [AF_INET, AF_INET6] {
        let mut bytes = 0;
        let mut buf = Vec::<u32>::new();
        loop {
            let ptr = if buf.is_empty() {
                None
            } else {
                Some(buf.as_mut_ptr().cast())
            };
            let rc = GetExtendedTcpTable(
                ptr,
                &mut bytes,
                false,
                family.0 as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
            if rc == ERROR_INSUFFICIENT_BUFFER.0 {
                buf.resize((bytes as usize).div_ceil(4), 0);
                continue;
            }
            if rc != 0 {
                return Err(Error::from_hresult(WIN32_ERROR(rc).to_hresult()));
            }
            break;
        }
        let (&count, body) = buf.split_first().ok_or(Error::from_hresult(E_UNEXPECTED))?;
        macro_rules! rows {
            ($ty:ty) => {{
                if (count as usize) > size_of_val(body) / size_of::<$ty>() {
                    return Err(Error::from_hresult(E_UNEXPECTED));
                }
                std::slice::from_raw_parts(body.as_ptr().cast::<$ty>(), count as usize)
            }};
        }
        if family == AF_INET {
            for row in rows!(MIB_TCPROW_OWNER_PID)
                .iter()
                .filter(|r| r.dwOwningPid == pid)
            {
                result.push(listener(
                    Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).into(),
                    row.dwLocalPort,
                    0,
                ));
            }
        } else {
            for row in rows!(MIB_TCP6ROW_OWNER_PID)
                .iter()
                .filter(|r| r.dwOwningPid == pid)
            {
                result.push(listener(
                    Ipv6Addr::from(row.ucLocalAddr).into(),
                    row.dwLocalPort,
                    row.dwLocalScopeId,
                ));
            }
        }
    }
    Ok(result)
}

unsafe fn inspect(pid: u32, own: &[usize]) -> Result<Value> {
    let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    GetProcessTimes(process.0, created, exited, kernel, user)?;
    let created = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
    let session_id = session(pid)?;
    let other = token_user(process.0)?;
    let sid = |buf: &[usize]| (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid;
    let same = EqualSid(sid(own), sid(&other)).is_ok();
    let mut path = vec![0u16; 32768];
    let mut len = path.len() as u32;
    QueryFullProcessImageNameW(
        process.0,
        PROCESS_NAME_WIN32,
        PWSTR(path.as_mut_ptr()),
        &mut len,
    )?;
    let name = String::from_utf16_lossy(&path[..len as usize]);
    let unc = name.starts_with(r"\\")
        && (!name.starts_with(r"\\?\") || name.to_ascii_lowercase().starts_with(r"\\?\unc\"));
    let wide = PCWSTR(path.as_ptr());
    let mut report = json!({"pid":pid,"session_id":session_id,"creation_time_filetime":created,
      "same_user_sid":same,"image_path":name,"is_unc":unc,"listeners":listeners(pid)?,
      "is_reparse_point":null,"signature_status":null,"signature_valid":null});
    if !unc {
        let attr = GetFileAttributesW(wide);
        if attr == INVALID_FILE_ATTRIBUTES {
            return Err(Error::from_win32());
        }
        let reparse = attr & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
        report["is_reparse_point"] = json!(reparse);
        if !reparse {
            let mut file = WINTRUST_FILE_INFO {
                cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
                pcwszFilePath: wide,
                ..Default::default()
            };
            let mut data = WINTRUST_DATA {
                cbStruct: size_of::<WINTRUST_DATA>() as u32,
                dwUIChoice: WTD_UI_NONE,
                fdwRevocationChecks: WTD_REVOKE_NONE,
                dwUnionChoice: WTD_CHOICE_FILE,
                Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
                dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
                ..Default::default()
            };
            let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
            let status = WinVerifyTrust(
                HWND::default(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            );
            report["signature_status"] = json!(status);
            report["signature_valid"] = json!(status == 0);
        }
    }
    Ok(report)
}

unsafe fn scan() -> Result<Value> {
    let pid = GetCurrentProcessId();
    let own = token_user(GetCurrentProcess())?;
    let snapshot = Handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut candidates = Vec::new();
    let mut next = Process32FirstW(snapshot.0, &mut entry);
    while next.is_ok() {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if ["language_server.exe", "language_server_windows_x64.exe"]
            .iter()
            .any(|n| name.eq_ignore_ascii_case(n))
        {
            candidates.push(inspect(entry.th32ProcessID, &own).unwrap_or_else(
                |e| json!({"pid":entry.th32ProcessID,"error":e.to_string(),"hresult":e.code().0}),
            ));
        }
        next = Process32NextW(snapshot.0, &mut entry);
    }
    if let Err(e) = next {
        if e.code() != ERROR_NO_MORE_FILES.to_hresult() {
            return Err(e);
        }
    }
    Ok(json!({"current_pid":pid,"current_session_id":session(pid)?,"candidates":candidates}))
}

fn main() {
    let start = Instant::now();
    let mut report =
        unsafe { scan() }.unwrap_or_else(|e| json!({"error":e.to_string(),"hresult":e.code().0}));
    report["elapsed_ms"] = json!(start.elapsed().as_millis());
    println!("{report}");
    if report.get("error").is_some() {
        std::process::exit(1);
    }
}
