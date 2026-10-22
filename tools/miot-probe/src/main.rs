use aes::Aes128;
use block_modes::{block_padding::Pkcs7, BlockMode, Cbc};
use serde_json::{json, Value};
use std::{
    io::{self, Write},
    net::{IpAddr, SocketAddr},
    str::FromStr,
    time::{Duration, Instant},
};
use tokio::{net::UdpSocket, time::timeout};
use zeroize::Zeroize;

type Aes128Cbc = Cbc<Aes128, Pkcs7>;
const DEVICE_PORT: u16 = 54321;
const WATCH_SECONDS: u64 = 180;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(2);

struct SecretToken([u8; 16]);

impl Drop for SecretToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("探测失败：{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut ip = String::new();
    print!("挂灯局域网 IP（不含令牌）: ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    io::stdin().read_line(&mut ip).map_err(|e| e.to_string())?;
    let address = parse_address(ip.trim())?;

    let mut token_text = rpassword::prompt_password("挂灯本地令牌（隐藏输入，不会保存或输出）: ")
        .map_err(|e| format!("无法读取隐藏输入：{e}"))?;
    let parsed_token = parse_token(&token_text);
    token_text.zeroize();
    let mut parsed_token = parsed_token?;
    let token = SecretToken(parsed_token);
    parsed_token.zeroize();

    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("无法创建局域网 UDP 连接：{e}"))?;
    socket
        .connect(address)
        .await
        .map_err(|e| format!("无法连接挂灯地址：{e}"))?;

    let (device_id, device_stamp) = handshake(&socket).await?;
    println!("MIoT 握手成功；开始最多 {WATCH_SECONDS} 秒只读状态观察。按遥控器完成三次关灯/开灯。按 Ctrl-C 可随时结束。");
    println!("令牌不会写入磁盘或日志；探测器只读取开关状态，不会控制灯或显示器。");

    let started = Instant::now();
    let mut stamp = device_stamp.wrapping_add(1);
    let mut last_state: Option<bool> = None;
    let mut transitions = 0_u8;
    let mut consecutive_failures = 0_u8;

    while started.elapsed() < Duration::from_secs(WATCH_SECONDS) && transitions < 6 {
        match read_light_state(&socket, device_id, stamp, &token.0).await {
            Ok(state) => {
                consecutive_failures = 0;
                if let Some(previous) = last_state {
                    if state != previous {
                        transitions = transitions.saturating_add(1);
                        println!(
                            "状态变化 {transitions}/6：{} → {}（距启动 {:.1}s）",
                            state_label(previous),
                            state_label(state),
                            started.elapsed().as_secs_f32()
                        );
                    }
                } else {
                    println!("初始状态：{}", state_label(state));
                }
                last_state = Some(state);
            }
            Err(error) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                if consecutive_failures >= 3 {
                    return Err(format!("连续 3 次读取失败，停止探测；{error}"));
                }
                eprintln!("状态读取失败（不会触发任何动作）：{error}");
            }
        }
        stamp = stamp.wrapping_add(1);
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    println!("观察结束：识别到 {transitions}/6 次变化。达到 6/6 才通过状态变化门槛。");
    Ok(())
}

fn parse_address(input: &str) -> Result<SocketAddr, String> {
    if let Ok(address) = SocketAddr::from_str(input) {
        return Ok(address);
    }
    input
        .parse::<IpAddr>()
        .map(|ip| SocketAddr::new(ip, DEVICE_PORT))
        .map_err(|_| "IP 地址格式无效；请输入 IPv4/IPv6 地址，可选显式端口".to_string())
}

fn parse_token(input: &str) -> Result<[u8; 16], String> {
    let value = input.trim();
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("令牌格式无效；需要 32 位十六进制字符".into());
    }
    let mut token = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let part = std::str::from_utf8(pair).map_err(|_| "令牌格式无效".to_string())?;
        token[index] = u8::from_str_radix(part, 16).map_err(|_| "令牌格式无效".to_string())?;
    }
    Ok(token)
}

async fn handshake(socket: &UdpSocket) -> Result<(u32, u32), String> {
    let mut request = [0xff; 32];
    request[..4].copy_from_slice(&[0x21, 0x31, 0x00, 0x20]);
    socket
        .send(&request)
        .await
        .map_err(|e| format!("发送 MIoT 握手失败：{e}"))?;

    let mut response = [0_u8; 256];
    let length = timeout(IO_TIMEOUT, socket.recv(&mut response))
        .await
        .map_err(|_| "握手超时；检查挂灯 IP、同一局域网和设备在线状态".to_string())?
        .map_err(|e| format!("接收 MIoT 握手失败：{e}"))?;
    if length < 32 || response[0..2] != [0x21, 0x31] {
        return Err("挂灯返回的数据不是有效 MIoT 握手响应".into());
    }
    let packet_length = u16::from_be_bytes([response[2], response[3]]) as usize;
    if packet_length != 32 || length < packet_length {
        return Err("挂灯握手响应长度无效".into());
    }
    let device_id = u32::from_be_bytes(response[8..12].try_into().unwrap());
    let stamp = u32::from_be_bytes(response[12..16].try_into().unwrap());
    if device_id == 0 || device_id == u32::MAX {
        return Err("挂灯握手没有提供有效设备编号".into());
    }
    Ok((device_id, stamp))
}

async fn read_light_state(
    socket: &UdpSocket,
    device_id: u32,
    stamp: u32,
    token: &[u8; 16],
) -> Result<bool, String> {
    let payload = json!({
        "id": stamp,
        "method": "get_properties",
        "params": [{ "did": "screenguard-lightbar-probe", "siid": 2, "piid": 1 }]
    })
    .to_string();
    let packet = encode_packet(device_id, stamp, token, payload.as_bytes())?;
    socket
        .send(&packet)
        .await
        .map_err(|e| format!("发送状态查询失败：{e}"))?;

    let mut response = [0_u8; 2048];
    let length = timeout(IO_TIMEOUT, socket.recv(&mut response))
        .await
        .map_err(|_| "状态查询超时；确认局域网令牌正确且设备支持本地 MIoT 读取".to_string())?
        .map_err(|e| format!("接收状态失败：{e}"))?;
    decode_light_state(&response[..length], token)
}

fn encode_packet(
    device_id: u32,
    stamp: u32,
    token: &[u8; 16],
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    let key = md5::compute(token).0;
    let mut iv_source = Vec::with_capacity(32);
    iv_source.extend_from_slice(&key);
    iv_source.extend_from_slice(token);
    let iv = md5::compute(iv_source).0;
    let cipher =
        Aes128Cbc::new_from_slices(&key, &iv).map_err(|_| "无法初始化 MIoT 加密器".to_string())?;
    let encrypted = cipher.encrypt_vec(payload);
    let mut packet = make_header(device_id, stamp, token, encrypted.len())?;
    packet.extend_from_slice(&encrypted);
    let digest = md5::compute(&packet);
    packet[16..32].copy_from_slice(&digest.0);
    Ok(packet)
}

fn make_header(
    device_id: u32,
    stamp: u32,
    checksum: &[u8; 16],
    payload_len: usize,
) -> Result<Vec<u8>, String> {
    let length = 32_usize
        .checked_add(payload_len)
        .filter(|n| *n <= u16::MAX as usize)
        .ok_or_else(|| "MIoT 数据包过大".to_string())?;
    let mut header = Vec::with_capacity(32 + payload_len);
    header.extend_from_slice(&[0x21, 0x31]);
    header.extend_from_slice(&(length as u16).to_be_bytes());
    header.extend_from_slice(&0_u32.to_be_bytes());
    header.extend_from_slice(&device_id.to_be_bytes());
    header.extend_from_slice(&stamp.to_be_bytes());
    header.extend_from_slice(checksum);
    Ok(header)
}

fn decode_light_state(packet: &[u8], token: &[u8; 16]) -> Result<bool, String> {
    if packet.len() < 32 || packet[0..2] != [0x21, 0x31] {
        return Err("响应头无效".into());
    }
    let packet_length = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if packet_length != packet.len() || packet_length <= 32 {
        return Err("响应长度无效或缺少加密内容".into());
    }
    let mut checksum_source = packet.to_vec();
    checksum_source[16..32].copy_from_slice(token);
    let expected = md5::compute(&checksum_source);
    if packet[16..32] != expected.0 {
        return Err("响应校验失败；令牌不匹配或设备响应无效".into());
    }

    let key = md5::compute(token).0;
    let mut iv_source = Vec::with_capacity(32);
    iv_source.extend_from_slice(&key);
    iv_source.extend_from_slice(token);
    let iv = md5::compute(iv_source).0;
    let cipher =
        Aes128Cbc::new_from_slices(&key, &iv).map_err(|_| "无法初始化 MIoT 解密器".to_string())?;
    let plaintext = cipher
        .decrypt_vec(&packet[32..])
        .map_err(|_| "无法解密响应；请检查设备令牌".to_string())?;
    let response: Value =
        serde_json::from_slice(&plaintext).map_err(|_| "设备响应不是有效 JSON".to_string())?;
    let result = response
        .get("result")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .ok_or_else(|| "响应没有包含 light.on 属性".to_string())?;
    if result.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("设备拒绝读取 light.on 属性".into());
    }
    result
        .get("value")
        .and_then(Value::as_bool)
        .ok_or_else(|| "light.on 返回值不是布尔状态".into())
}

fn state_label(state: bool) -> &'static str {
    if state {
        "开灯"
    } else {
        "关灯"
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_light_state, encode_packet, parse_address, parse_token};
    use aes::Aes128;
    use block_modes::{block_padding::Pkcs7, BlockMode, Cbc};
    use serde_json::json;

    type Aes128Cbc = Cbc<Aes128, Pkcs7>;

    #[test]
    fn parses_token_without_logging_or_persisting_it() {
        assert_eq!(
            parse_token("00112233445566778899aabbccddeeff").unwrap()[0],
            0
        );
        assert!(parse_token("not-a-token").is_err());
    }

    #[test]
    fn parses_ip_with_default_or_explicit_port() {
        assert_eq!(parse_address("192.168.1.4").unwrap().port(), 54321);
        assert_eq!(parse_address("192.168.1.4:1234").unwrap().port(), 1234);
        assert_eq!(parse_address("::1").unwrap().port(), 54321);
        assert_eq!(parse_address("[::1]:1234").unwrap().port(), 1234);
        assert!(parse_address("bad-address").is_err());
    }

    #[test]
    fn encodes_and_reads_miot_light_state_response() {
        let token = [0x11_u8; 16];
        let payload = json!({"id": 1, "result": [{"code": 0, "value": true}]}).to_string();
        let request = encode_packet(42, 1, &token, payload.as_bytes()).unwrap();
        let encrypted = request[32..].to_vec();
        let key = md5::compute(token).0;
        let mut iv_source = Vec::new();
        iv_source.extend_from_slice(&key);
        iv_source.extend_from_slice(&token);
        let iv = md5::compute(iv_source).0;
        let ciphertext = Aes128Cbc::new_from_slices(&key, &iv)
            .unwrap()
            .encrypt_vec(br#"{"id":2,"result":[{"code":0,"value":false}]}"#);
        let mut response = super::make_header(42, 2, &token, ciphertext.len()).unwrap();
        response.extend_from_slice(&ciphertext);
        let checksum = md5::compute(&response);
        response[16..32].copy_from_slice(&checksum.0);
        assert!(decode_light_state(&response, &token).unwrap() == false);
        assert!(!encrypted.is_empty());
    }
}
