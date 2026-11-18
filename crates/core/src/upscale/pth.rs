//! Turning Real-ESRGAN's compact models from PyTorch checkpoints into ncnn
//! models that realesrgan-ncnn-vulkan can run.
//!
//! Some Real-ESRGAN models (`realesr-general-x4v3` among them) are published
//! only as PyTorch `.pth` files. Rather than redistribute converted copies,
//! CatConvert downloads the original and converts it here, without Python.
//!
//! A `.pth` file is a zip archive holding `<prefix>/data.pkl`, a pickle that
//! describes the state dict, and `<prefix>/data/<key>`, one raw little-endian
//! storage per tensor. The pickle is read by a tiny interpreter that knows only
//! the handful of opcodes and Python callables `torch.save` uses for a state
//! dict, and builds nothing else: no code from the file is ever run.
//!
//! The network is `SRVGGNetCompact`: a 3×3 convolution and PReLU, a chain of
//! further convolution + PReLU pairs, a last convolution to `3 × upscale²`
//! channels, a pixel shuffle, and the input upsampled (nearest neighbour)
//! added on top. The ncnn graph written here mirrors the official conversions
//! shipped with realesrgan-ncnn-vulkan, layer for layer, with the weights
//! stored as half floats like theirs.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};

/// An ncnn model: the text `.param` and the binary `.bin` weights.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NcnnModel {
    pub param: String,
    pub bin: Vec<u8>,
}

/// Convert a Real-ESRGAN `SRVGGNetCompact` checkpoint (`.pth`, as published
/// with xinntao/Real-ESRGAN) into ncnn's format, with the input blob named `data`
/// and the output blob named `output`, as realesrgan-ncnn-vulkan expects.
pub(crate) fn compact_to_ncnn(pth: &[u8]) -> Result<NcnnModel, String> {
    let mut checkpoint = Checkpoint::open(pth)?;
    let pickle = checkpoint.pickle()?;
    let state = Unpickler::new(&pickle).run()?.into_state_dict()?;
    let net = Compact::load(&state, &mut checkpoint)?;
    Ok(net.to_ncnn())
}

// ---------------------------------------------------------------------------
// The zip container

/// Largest `data.pkl` accepted. Real state dicts take a few kilobytes.
const MAX_PICKLE: u64 = 4 << 20;

/// An opened `.pth` archive.
struct Checkpoint<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// Everything before `data.pkl` in its entry name, e.g. `archive/`.
    prefix: String,
    /// Storage bytes that may still be read. Starts at the file size: torch
    /// stores entries uncompressed, so honest storages can never add up to more.
    budget: u64,
    /// Storages read so far, by key, so shared ones are read only once.
    storages: HashMap<String, Vec<u8>>,
}

impl<'a> Checkpoint<'a> {
    fn open(pth: &'a [u8]) -> Result<Self, String> {
        let zip = zip::ZipArchive::new(Cursor::new(pth))
            .map_err(|_| "not a PyTorch checkpoint".to_owned())?;
        let prefix = find_prefix(&zip)?;
        let mut checkpoint = Self {
            zip,
            prefix,
            budget: pth.len() as u64,
            storages: HashMap::new(),
        };
        checkpoint.check_byte_order()?;
        Ok(checkpoint)
    }

    /// The pickled state dict.
    fn pickle(&mut self) -> Result<Vec<u8>, String> {
        let name = format!("{}data.pkl", self.prefix);
        self.read(&name, MAX_PICKLE)
    }

    /// Storages are little-endian unless a `byteorder` entry says otherwise.
    fn check_byte_order(&mut self) -> Result<(), String> {
        let name = format!("{}byteorder", self.prefix);
        if self.zip.index_for_name(&name).is_none() {
            return Ok(());
        }
        match self.read(&name, 16)?.trim_ascii() {
            b"little" => Ok(()),
            _ => Err("big-endian checkpoints are not supported".to_owned()),
        }
    }

    /// Read a whole entry, refusing ones larger than `limit`.
    fn read(&mut self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        let entry = self
            .zip
            .by_name(name)
            .map_err(|_| format!("checkpoint has no {name}"))?;
        let size = entry.size();
        if size > limit {
            return Err(format!("{name} in the checkpoint is too large"));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        entry
            .take(size)
            .read_to_end(&mut bytes)
            .map_err(|_| format!("cannot read {name} from the checkpoint"))?;
        if bytes.len() as u64 != size {
            return Err(format!("{name} in the checkpoint is truncated"));
        }
        Ok(bytes)
    }

    /// The raw bytes of the storage named `key`.
    fn storage(&mut self, key: &str) -> Result<&[u8], String> {
        if !self.storages.contains_key(key) {
            let name = format!("{}data/{key}", self.prefix);
            let bytes = self.read(&name, self.budget)?;
            self.budget -= bytes.len() as u64;
            self.storages.insert(key.to_owned(), bytes);
        }
        Ok(&self.storages[key])
    }

    /// A tensor's values, which must be laid out contiguously in its storage.
    fn tensor(&mut self, tensor: &Tensor) -> Result<Vec<f32>, String> {
        let numel = tensor.numel()?;
        if tensor.stride != contiguous_strides(&tensor.shape) {
            return Err("non-contiguous tensors are not supported".to_owned());
        }
        let size = tensor.dtype.size();
        let start = tensor.offset.checked_mul(size);
        let end = tensor
            .offset
            .checked_add(numel)
            .filter(|&end| end <= tensor.storage_len)
            .and_then(|end| end.checked_mul(size));
        let bytes = self.storage(&tensor.key)?;
        let data = start
            .zip(end)
            .and_then(|(start, end)| {
                bytes.get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)
            })
            .ok_or("tensor data is truncated")?;
        Ok(match tensor.dtype {
            DType::F32 => data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&b| f32::from_le_bytes(b))
                .collect(),
            DType::F16 => data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| f16_to_f32(u16::from_le_bytes(b)))
                .collect(),
        })
    }
}

/// The directory `data.pkl` sits in, with a trailing slash (or nothing).
fn find_prefix(zip: &zip::ZipArchive<Cursor<&[u8]>>) -> Result<String, String> {
    let mut found = None;
    for name in zip.file_names().flatten() {
        let Some(prefix) = name.strip_suffix("data.pkl") else {
            continue;
        };
        let top_level =
            prefix.is_empty() || (prefix.ends_with('/') && prefix.matches('/').count() == 1);
        if top_level && found.replace(prefix.to_owned()).is_some() {
            return Err("checkpoint holds more than one data.pkl".to_owned());
        }
    }
    found.ok_or_else(|| "not a PyTorch checkpoint".to_owned())
}

/// Row-major strides for `shape`.
fn contiguous_strides(shape: &[u64]) -> Vec<u64> {
    let mut strides = vec![1; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() {
        strides[i] = strides[i + 1] * shape[i + 1].max(1);
    }
    strides
}

// ---------------------------------------------------------------------------
// The pickle

/// Element types a storage may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DType {
    F32,
    F16,
}

impl DType {
    fn size(self) -> u64 {
        match self {
            Self::F32 => 4,
            Self::F16 => 2,
        }
    }
}

/// The Python callables and classes a state dict pickle may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Global {
    OrderedDict,
    RebuildTensor,
    RebuildParameter,
    Storage(DType),
}

impl Global {
    fn resolve(module: &str, name: &str) -> Result<Self, String> {
        Ok(match (module, name) {
            ("collections", "OrderedDict") => Self::OrderedDict,
            ("torch._utils", "_rebuild_tensor_v2") => Self::RebuildTensor,
            ("torch._utils", "_rebuild_parameter") => Self::RebuildParameter,
            ("torch", "FloatStorage") => Self::Storage(DType::F32),
            ("torch", "HalfStorage") => Self::Storage(DType::F16),
            _ => return Err(format!("unsupported object in checkpoint: {module}.{name}")),
        })
    }
}

/// A tensor as the pickle describes it: where its values are, not the values.
#[derive(Debug, Clone, PartialEq)]
struct Tensor {
    key: String,
    dtype: DType,
    /// Elements in the whole storage.
    storage_len: u64,
    /// Offset into the storage, in elements.
    offset: u64,
    shape: Vec<u64>,
    stride: Vec<u64>,
}

impl Tensor {
    fn numel(&self) -> Result<u64, String> {
        self.shape
            .iter()
            .try_fold(1u64, |n, &d| n.checked_mul(d))
            .ok_or_else(|| "tensor is too large".to_owned())
    }
}

/// Index of an object in [`Unpickler::objects`].
type Id = usize;

/// A Python object, as far as a state dict needs one.
#[derive(Debug, Clone, PartialEq)]
enum Object {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Tuple(Vec<Id>),
    List(Vec<Id>),
    Dict(Vec<(Id, Id)>),
    Global(Global),
    Storage { key: String, dtype: DType, len: u64 },
    Tensor(Tensor),
}

/// A minimal pickle interpreter for `torch.save`d state dicts.
///
/// Objects live in one flat list and refer to each other by index, so shared
/// and even cyclic references cost nothing and nothing recurses. Every opcode
/// creates at most one object, which bounds memory by the pickle's size.
struct Unpickler<'a> {
    data: &'a [u8],
    pos: usize,
    objects: Vec<Object>,
    stack: Vec<Id>,
    /// Stack heights at each open MARK.
    marks: Vec<usize>,
    memo: HashMap<u32, Id>,
}

impl<'a> Unpickler<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            objects: Vec::new(),
            stack: Vec::new(),
            marks: Vec::new(),
            memo: HashMap::new(),
        }
    }

    /// Run the pickle to its STOP and return the unpickled objects.
    fn run(mut self) -> Result<Unpickled, String> {
        loop {
            let op = self.take(1)?[0];
            if op == b'.' {
                let root = self.pop()?;
                return Ok(Unpickled {
                    objects: self.objects,
                    root,
                });
            }
            self.step(op)?;
        }
    }

    fn step(&mut self, op: u8) -> Result<(), String> {
        match op {
            0x80 => {
                if self.take(1)?[0] > 5 {
                    return Err("unsupported pickle protocol".to_owned());
                }
            }
            0x95 => {
                self.take(8)?; // FRAME: only a hint for buffered readers.
            }
            b'}' => self.push(Object::Dict(Vec::new())),
            b']' => self.push(Object::List(Vec::new())),
            b')' => self.push(Object::Tuple(Vec::new())),
            b'(' => self.marks.push(self.stack.len()),
            b'X' => {
                let len = self.u32()? as usize;
                self.string(len)?;
            }
            0x8c => {
                let len = usize::from(self.take(1)?[0]);
                self.string(len)?;
            }
            b'c' => {
                let module = self.line()?;
                let name = self.line()?;
                let global = Global::resolve(&module, &name)?;
                self.push(Object::Global(global));
            }
            0x93 => {
                let name = self.pop_str()?;
                let module = self.pop_str()?;
                let global = Global::resolve(&module, &name)?;
                self.push(Object::Global(global));
            }
            b'Q' => {
                let pid = self.pop()?;
                let storage = self.persistent_load(pid)?;
                self.push(storage);
            }
            b't' => {
                let items = self.pop_mark()?;
                self.push(Object::Tuple(items));
            }
            0x85..=0x87 => {
                let n = usize::from(op - 0x84);
                let at = self
                    .stack
                    .len()
                    .checked_sub(n)
                    .ok_or("pickle stack underflow")?;
                let items = self.stack.split_off(at);
                self.push(Object::Tuple(items));
            }
            b'J' => {
                let b = self.take(4)?;
                let value = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                self.push(Object::Int(value.into()));
            }
            b'K' => {
                let value = self.take(1)?[0];
                self.push(Object::Int(value.into()));
            }
            b'M' => {
                let b = self.take(2)?;
                self.push(Object::Int(u16::from_le_bytes([b[0], b[1]]).into()));
            }
            0x8a => {
                let len = usize::from(self.take(1)?[0]);
                let value = long(self.take(len)?)?;
                self.push(Object::Int(value));
            }
            b'G' => {
                let b = self.take(8)?;
                let value = f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                self.push(Object::Float(value));
            }
            0x88 => self.push(Object::Bool(true)),
            0x89 => self.push(Object::Bool(false)),
            b'N' => self.push(Object::None),
            b'R' => {
                let args = self.pop()?;
                let callable = self.pop()?;
                let result = self.call(callable, args)?;
                self.stack.push(result);
            }
            b'b' => {
                // BUILD sets attributes, e.g. a state dict's `_metadata`,
                // which the conversion has no use for.
                self.pop()?;
                self.top()?;
            }
            b's' => {
                let value = self.pop()?;
                let key = self.pop()?;
                self.dict_mut()?.push((key, value));
            }
            b'u' => {
                let items = self.pop_mark()?;
                let (pairs, odd) = items.as_chunks::<2>();
                if !odd.is_empty() {
                    return Err("odd number of items for a dict in pickle".to_owned());
                }
                let pairs = pairs.iter().map(|&[key, value]| (key, value));
                self.dict_mut()?.extend(pairs);
            }
            b'a' => {
                let value = self.pop()?;
                self.list_mut()?.push(value);
            }
            b'e' => {
                let items = self.pop_mark()?;
                self.list_mut()?.extend(items);
            }
            b'q' => {
                let index = self.take(1)?[0].into();
                self.memoize(index)?;
            }
            b'r' => {
                let index = self.u32()?;
                self.memoize(index)?;
            }
            0x94 => {
                let index = u32::try_from(self.memo.len()).map_err(|_| "pickle memo overflow")?;
                self.memoize(index)?;
            }
            b'h' => {
                let index = self.take(1)?[0].into();
                self.recall(index)?;
            }
            b'j' => {
                let index = self.u32()?;
                self.recall(index)?;
            }
            _ => return Err(format!("unsupported pickle opcode 0x{op:02x}")),
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let data = self.data;
        let bytes = self
            .pos
            .checked_add(n)
            .and_then(|end| data.get(self.pos..end))
            .ok_or("truncated pickle")?;
        self.pos += n;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A newline-terminated ASCII line, as GLOBAL uses.
    fn line(&mut self) -> Result<String, String> {
        let rest = &self.data[self.pos..];
        let len = rest
            .iter()
            .position(|&b| b == b'\n')
            .ok_or("truncated pickle")?;
        let line = self.take(len + 1)?;
        std::str::from_utf8(&line[..len])
            .map(str::to_owned)
            .map_err(|_| "invalid name in pickle".to_owned())
    }

    fn string(&mut self, len: usize) -> Result<(), String> {
        let bytes = self.take(len)?;
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid string in pickle")?;
        self.push(Object::Str(text.to_owned()));
        Ok(())
    }

    fn push(&mut self, object: Object) {
        self.objects.push(object);
        self.stack.push(self.objects.len() - 1);
    }

    fn pop(&mut self) -> Result<Id, String> {
        if self.marks.last() == Some(&self.stack.len()) {
            return Err("pickle stack underflow".to_owned());
        }
        self.stack
            .pop()
            .ok_or_else(|| "pickle stack underflow".to_owned())
    }

    fn top(&self) -> Result<Id, String> {
        self.stack
            .last()
            .copied()
            .ok_or_else(|| "pickle stack underflow".to_owned())
    }

    fn pop_str(&mut self) -> Result<String, String> {
        let id = self.pop()?;
        match &self.objects[id] {
            Object::Str(text) => Ok(text.clone()),
            _ => Err("expected a string in pickle".to_owned()),
        }
    }

    /// Everything pushed since the last MARK.
    fn pop_mark(&mut self) -> Result<Vec<Id>, String> {
        let at = self.marks.pop().ok_or("pickle has no MARK")?;
        Ok(self.stack.split_off(at))
    }

    fn dict_mut(&mut self) -> Result<&mut Vec<(Id, Id)>, String> {
        let top = self.top()?;
        match &mut self.objects[top] {
            Object::Dict(items) => Ok(items),
            _ => Err("expected a dict in pickle".to_owned()),
        }
    }

    fn list_mut(&mut self) -> Result<&mut Vec<Id>, String> {
        let top = self.top()?;
        match &mut self.objects[top] {
            Object::List(items) => Ok(items),
            _ => Err("expected a list in pickle".to_owned()),
        }
    }

    fn memoize(&mut self, index: u32) -> Result<(), String> {
        let top = self.top()?;
        self.memo.insert(index, top);
        Ok(())
    }

    fn recall(&mut self, index: u32) -> Result<(), String> {
        let id = *self
            .memo
            .get(&index)
            .ok_or("pickle refers to a missing memo entry")?;
        self.stack.push(id);
        Ok(())
    }

    /// `('storage', torch.FloatStorage, key, location, numel)` → a storage.
    fn persistent_load(&self, pid: Id) -> Result<Object, String> {
        let invalid = || "invalid storage reference in pickle".to_owned();
        let Object::Tuple(items) = &self.objects[pid] else {
            return Err(invalid());
        };
        let [kind, class, key, _location, len] = items.as_slice() else {
            return Err(invalid());
        };
        let (Object::Str(kind), Object::Global(Global::Storage(dtype)), Object::Str(key)) = (
            &self.objects[*kind],
            &self.objects[*class],
            &self.objects[*key],
        ) else {
            return Err(invalid());
        };
        if kind != "storage" {
            return Err(invalid());
        }
        Ok(Object::Storage {
            key: key.clone(),
            dtype: *dtype,
            len: self.uint(*len)?,
        })
    }

    /// REDUCE: the few calls a state dict makes.
    fn call(&mut self, callable: Id, args: Id) -> Result<Id, String> {
        let Object::Tuple(args) = &self.objects[args] else {
            return Err("invalid call in pickle".to_owned());
        };
        match (&self.objects[callable], args.as_slice()) {
            (Object::Global(Global::OrderedDict), []) => {
                self.objects.push(Object::Dict(Vec::new()));
                Ok(self.objects.len() - 1)
            }
            (Object::Global(Global::RebuildTensor), [storage, offset, shape, stride, ..]) => {
                let tensor = self.rebuild_tensor(*storage, *offset, *shape, *stride)?;
                self.objects.push(Object::Tensor(tensor));
                Ok(self.objects.len() - 1)
            }
            (Object::Global(Global::RebuildParameter), [tensor, ..])
                if matches!(self.objects[*tensor], Object::Tensor(_)) =>
            {
                Ok(*tensor)
            }
            _ => Err("unsupported call in pickle".to_owned()),
        }
    }

    fn rebuild_tensor(
        &self,
        storage: Id,
        offset: Id,
        shape: Id,
        stride: Id,
    ) -> Result<Tensor, String> {
        let Object::Storage { key, dtype, len } = &self.objects[storage] else {
            return Err("tensor without a storage in pickle".to_owned());
        };
        Ok(Tensor {
            key: key.clone(),
            dtype: *dtype,
            storage_len: *len,
            offset: self.uint(offset)?,
            shape: self.uints(shape)?,
            stride: self.uints(stride)?,
        })
    }

    fn uint(&self, id: Id) -> Result<u64, String> {
        match self.objects[id] {
            Object::Int(value) => {
                u64::try_from(value).map_err(|_| "negative size in pickle".to_owned())
            }
            _ => Err("expected an integer in pickle".to_owned()),
        }
    }

    fn uints(&self, id: Id) -> Result<Vec<u64>, String> {
        match &self.objects[id] {
            Object::Tuple(items) => items.iter().map(|&item| self.uint(item)).collect(),
            _ => Err("expected a tuple in pickle".to_owned()),
        }
    }
}

/// A LONG1 integer: little-endian two's complement, at most 8 bytes here.
fn long(bytes: &[u8]) -> Result<i64, String> {
    if bytes.len() > 8 {
        return Err("integer in pickle is too large".to_owned());
    }
    let fill = if bytes.last().is_some_and(|&b| b & 0x80 != 0) {
        0xFF
    } else {
        0
    };
    let mut buf = [fill; 8];
    buf[..bytes.len()].copy_from_slice(bytes);
    Ok(i64::from_le_bytes(buf))
}

/// What a pickle unpickled to.
#[derive(Debug)]
struct Unpickled {
    objects: Vec<Object>,
    root: Id,
}

impl Unpickled {
    /// The tensors by name. Real-ESRGAN wraps the state dict in a dict under
    /// `params_ema` or `params` (preferring `params_ema`, as its own inference
    /// code does); a bare state dict works too.
    fn into_state_dict(self) -> Result<Vec<(String, Tensor)>, String> {
        let root = self
            .dict(self.root)
            .ok_or("checkpoint is not a state dict")?;
        let state = ["params_ema", "params"]
            .iter()
            .find_map(|name| self.lookup(root, name))
            .unwrap_or(self.root);
        let items = self.dict(state).ok_or("checkpoint is not a state dict")?;
        items
            .iter()
            .map(
                |&(key, value)| match (&self.objects[key], &self.objects[value]) {
                    (Object::Str(key), Object::Tensor(tensor)) => Ok((key.clone(), tensor.clone())),
                    (Object::Str(key), _) => Err(format!("unexpected entry in state dict: {key}")),
                    _ => Err("unexpected entry in state dict".to_owned()),
                },
            )
            .collect()
    }

    fn dict(&self, id: Id) -> Option<&[(Id, Id)]> {
        match &self.objects[id] {
            Object::Dict(items) => Some(items),
            _ => None,
        }
    }

    /// The dict stored under `name` in `items`, if there is one.
    fn lookup(&self, items: &[(Id, Id)], name: &str) -> Option<Id> {
        items.iter().rev().find_map(|&(key, value)| {
            let named = matches!(&self.objects[key], Object::Str(key) if key == name);
            (named && self.dict(value).is_some()).then_some(value)
        })
    }
}

// ---------------------------------------------------------------------------
// The network

/// Largest upscale factor accepted (the published models are 2× to 4×).
const MAX_UPSCALE: u64 = 8;

/// One 3×3 convolution's weights.
#[derive(Debug)]
struct Conv {
    outputs: usize,
    weight: Vec<f32>,
    bias: Vec<f32>,
}

/// A loaded `SRVGGNetCompact`.
#[derive(Debug)]
struct Compact {
    /// The first, the `num_conv` body convolutions, and the last.
    convs: Vec<Conv>,
    /// One PReLU after every convolution but the last.
    prelus: Vec<Vec<f32>>,
    upscale: u64,
}

/// A layer's `weight` and `bias` from the state dict.
#[derive(Debug, Default)]
struct Params<'a> {
    weight: Option<&'a Tensor>,
    bias: Option<&'a Tensor>,
}

impl Compact {
    /// Check the layer layout against `SRVGGNetCompact`, work out its size from
    /// the tensor shapes, and read the weights.
    fn load(state: &[(String, Tensor)], checkpoint: &mut Checkpoint) -> Result<Self, String> {
        let body = body_layers(state)?;
        let count = body.len();
        if count < 3 || count % 2 == 0 {
            return Err(format!("unexpected layer layout: {count} layers in body"));
        }
        let first = body[0]
            .weight
            .ok_or("unexpected layer layout: no body.0.weight")?;
        let feat = match first.shape.as_slice() {
            [feat, 3, 3, 3] if *feat > 0 => *feat,
            shape => {
                return Err(format!(
                    "unexpected layer layout: body.0.weight is {shape:?}"
                ));
            }
        };
        let last = body[count - 1].weight.map(|t| t.shape.as_slice());
        let upscale = match last {
            Some(&[out, inputs, 3, 3]) if inputs == feat => upscale_for(out),
            _ => None,
        }
        .ok_or_else(|| {
            format!(
                "unexpected layer layout: body.{} is not 3 × upscale² outputs",
                count - 1
            )
        })?;

        let mut convs = Vec::new();
        let mut prelus = Vec::new();
        for (index, params) in body.iter().enumerate() {
            if index % 2 == 0 {
                let inputs = if index == 0 { 3 } else { feat };
                let outputs = if index == count - 1 {
                    3 * upscale * upscale
                } else {
                    feat
                };
                convs.push(load_conv(checkpoint, index, params, inputs, outputs)?);
            } else {
                prelus.push(load_prelu(checkpoint, index, params, feat)?);
            }
        }
        Ok(Self {
            convs,
            prelus,
            upscale,
        })
    }

    /// The ncnn graph, laid out like the official realesrgan-ncnn-vulkan
    /// conversions: weights in layer order, convolution kernels as half
    /// floats behind ncnn's fp16 tag, biases and PReLU slopes as floats.
    fn to_ncnn(&self) -> NcnnModel {
        let mut graph = Graph::default();
        graph.layer("Input", "input", &[], &["data"], "");
        graph.layer(
            "Split",
            "splitncnn_input0",
            &["data"],
            &["data_splitncnn_0", "data_splitncnn_1"],
            "",
        );
        let mut bin = Vec::new();
        let mut blob = "data_splitncnn_1".to_owned();
        for (index, conv) in self.convs.iter().enumerate() {
            let name = format!("conv_{index}");
            let params = format!("0={} 1=3 4=1 5=1 6={}", conv.outputs, conv.weight.len());
            graph.layer("Convolution", &name, &[&blob], &[&name], &params);
            write_half_weights(&mut bin, &conv.weight);
            write_floats(&mut bin, &conv.bias);
            blob = name;
            if let Some(slopes) = self.prelus.get(index) {
                let name = format!("prelu_{index}");
                let params = format!("0={}", slopes.len());
                graph.layer("PReLU", &name, &[&blob], &[&name], &params);
                write_floats(&mut bin, slopes);
                blob = name;
            }
        }
        let shuffle = format!("0={}", self.upscale);
        graph.layer(
            "PixelShuffle",
            "pixel_shuffle",
            &[&blob],
            &["pixel_shuffle"],
            &shuffle,
        );
        let scale = c_float(self.upscale as f32);
        let resize = format!("0=1 1={scale} 2={scale}");
        graph.layer(
            "Interp",
            "upsample",
            &["data_splitncnn_0"],
            &["upsample"],
            &resize,
        );
        graph.layer(
            "BinaryOp",
            "add",
            &["pixel_shuffle", "upsample"],
            &["output"],
            "",
        );
        NcnnModel {
            param: graph.finish(),
            bin,
        }
    }
}

/// Group `body.<index>.weight` / `.bias` by index. Anything else in the state
/// dict means it is some other network.
fn body_layers(state: &[(String, Tensor)]) -> Result<Vec<Params<'_>>, String> {
    let mut layers: BTreeMap<usize, Params> = BTreeMap::new();
    for (key, tensor) in state {
        let unexpected = || format!("unexpected layer layout: {key}");
        let rest = key.strip_prefix("body.").ok_or_else(unexpected)?;
        let (index, kind) = rest.split_once('.').ok_or_else(unexpected)?;
        let index: usize = index.parse().map_err(|_| unexpected())?;
        if index >= state.len() {
            return Err(unexpected());
        }
        let params = layers.entry(index).or_default();
        let slot = match kind {
            "weight" => &mut params.weight,
            "bias" => &mut params.bias,
            _ => return Err(unexpected()),
        };
        if slot.replace(tensor).is_some() {
            return Err(format!("unexpected layer layout: {key} twice"));
        }
    }
    let count = layers.len();
    if layers.keys().copied().ne(0..count) {
        return Err(
            "unexpected layer layout: body layers are not numbered 0, 1, 2, ...".to_owned(),
        );
    }
    Ok(layers.into_values().collect())
}

/// `out` = 3 × upscale² → upscale.
fn upscale_for(out: u64) -> Option<u64> {
    (1..=MAX_UPSCALE).find(|u| 3 * u * u == out)
}

fn load_conv(
    checkpoint: &mut Checkpoint,
    index: usize,
    params: &Params,
    inputs: u64,
    outputs: u64,
) -> Result<Conv, String> {
    let weight = expect(params.weight, index, "weight", &[outputs, inputs, 3, 3])?;
    let bias = expect(params.bias, index, "bias", &[outputs])?;
    Ok(Conv {
        outputs: outputs as usize,
        weight: checkpoint.tensor(weight)?,
        bias: checkpoint.tensor(bias)?,
    })
}

fn load_prelu(
    checkpoint: &mut Checkpoint,
    index: usize,
    params: &Params,
    feat: u64,
) -> Result<Vec<f32>, String> {
    if params.bias.is_some() {
        return Err(format!("unexpected layer layout: body.{index}.bias"));
    }
    let weight = expect(params.weight, index, "weight", &[feat])?;
    checkpoint.tensor(weight)
}

/// The tensor, if it is there and has the expected shape.
fn expect<'a>(
    tensor: Option<&'a Tensor>,
    index: usize,
    kind: &str,
    shape: &[u64],
) -> Result<&'a Tensor, String> {
    match tensor {
        Some(tensor) if tensor.shape == shape => Ok(tensor),
        Some(tensor) => Err(format!(
            "unexpected layer layout: body.{index}.{kind} is {:?}, expected {shape:?}",
            tensor.shape
        )),
        None => Err(format!("unexpected layer layout: no body.{index}.{kind}")),
    }
}

// ---------------------------------------------------------------------------
// ncnn output

/// ncnn's magic number on the first line of a `.param` file.
const PARAM_MAGIC: u32 = 7_767_517;
/// Tag in front of weights stored as half floats in a `.bin` file.
const FP16_TAG: u32 = 0x0130_6B47;

/// The layers of an ncnn `.param` file, written as they are added.
#[derive(Debug, Default)]
struct Graph {
    lines: String,
    layers: usize,
    blobs: usize,
}

impl Graph {
    fn layer(&mut self, kind: &str, name: &str, inputs: &[&str], outputs: &[&str], params: &str) {
        use std::fmt::Write as _;
        let _ = write!(
            self.lines,
            "{kind:<24} {name:<24} {} {}",
            inputs.len(),
            outputs.len()
        );
        for blob in inputs.iter().chain(outputs) {
            let _ = write!(self.lines, " {blob}");
        }
        if !params.is_empty() {
            let _ = write!(self.lines, " {params}");
        }
        self.lines.push('\n');
        self.layers += 1;
        self.blobs += outputs.len();
    }

    fn finish(self) -> String {
        format!(
            "{PARAM_MAGIC}\n{} {}\n{}",
            self.layers, self.blobs, self.lines
        )
    }
}

/// Floats as ncnn's half-float weight blob: the tag, then the values, padded
/// to four bytes.
fn write_half_weights(bin: &mut Vec<u8>, values: &[f32]) {
    bin.extend_from_slice(&FP16_TAG.to_le_bytes());
    for &value in values {
        bin.extend_from_slice(&f32_to_f16(value).to_le_bytes());
    }
    if !values.len().is_multiple_of(2) {
        bin.extend_from_slice(&[0, 0]);
    }
}

/// Raw little-endian floats, as ncnn reads biases and PReLU slopes.
fn write_floats(bin: &mut Vec<u8>, values: &[f32]) {
    for &value in values {
        bin.extend_from_slice(&value.to_le_bytes());
    }
}

/// A float the way C's `%e` prints it, as ncnn's own tools write them:
/// `4.000000e+00`.
fn c_float(value: f32) -> String {
    let text = format!("{value:.6e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.abs())
}

/// IEEE half → single precision.
fn f16_to_f32(half: u16) -> f32 {
    let sign = if half & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((half >> 10) & 0x1F);
    let fraction = f32::from(half & 0x3FF);
    sign * match exponent {
        0 => fraction * 2f32.powi(-24),
        0x1F if fraction == 0.0 => f32::INFINITY,
        0x1F => f32::NAN,
        _ => (1.0 + fraction / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// Single → IEEE half precision the way ncnn's model tools convert weights:
/// the fraction truncated and results too small for a normal half flushed to
/// zero. Rounding to nearest would be a hair more accurate, but this makes a
/// conversion byte for byte what ncnn's own converter writes, which is how it
/// was checked against the official models.
fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let fraction = bits & 0x7F_FFFF;
    if exponent == 0xFF {
        let nan = if fraction != 0 { 0x200 } else { 0 };
        return sign | 0x7C00 | nan;
    }
    match exponent - 127 + 15 {
        ..=0 => sign,
        0x1F.. => sign | 0x7C00,
        half_exponent => sign | ((half_exponent as u16) << 10) | (fraction >> 13) as u16,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Pickle opcodes, written the way `torch.save` writes them.
    #[derive(Default)]
    struct Pickle(Vec<u8>);

    impl Pickle {
        fn op(&mut self, bytes: &[u8]) -> &mut Self {
            self.0.extend_from_slice(bytes);
            self
        }

        fn str(&mut self, text: &str) -> &mut Self {
            self.op(b"X");
            self.op(&(text.len() as u32).to_le_bytes());
            self.op(text.as_bytes())
        }

        fn int(&mut self, value: u64) -> &mut Self {
            match value {
                0..=0xFF => self.op(&[b'K', value as u8]),
                0x100..=0xFFFF => self.op(b"M").op(&(value as u16).to_le_bytes()),
                _ => self.op(b"J").op(&(value as i32).to_le_bytes()),
            }
        }

        fn global(&mut self, module: &str, name: &str) -> &mut Self {
            self.op(format!("c{module}\n{name}\n").as_bytes())
        }

        fn tuple(&mut self, values: &[u64]) -> &mut Self {
            self.op(b"(");
            for &value in values {
                self.int(value);
            }
            self.op(b"t")
        }
    }

    /// A tensor for a fake checkpoint.
    struct Fake {
        name: String,
        shape: Vec<u64>,
        half: bool,
    }

    fn fake(name: &str, shape: &[u64]) -> Fake {
        Fake {
            name: name.to_owned(),
            shape: shape.to_vec(),
            half: false,
        }
    }

    /// The layers of a tiny compact net: 2 features, one body convolution, 2×.
    fn tiny() -> Vec<Fake> {
        vec![
            fake("body.0.weight", &[2, 3, 3, 3]),
            fake("body.0.bias", &[2]),
            fake("body.1.weight", &[2]),
            fake("body.2.weight", &[2, 2, 3, 3]),
            fake("body.2.bias", &[2]),
            fake("body.3.weight", &[2]),
            fake("body.4.weight", &[12, 2, 3, 3]),
            fake("body.4.bias", &[12]),
        ]
    }

    /// Values exactly representable as half floats, different for each tensor.
    fn values(index: usize, count: u64) -> Vec<f32> {
        (0..count)
            .map(|i| ((i as usize * 7 + index * 3) % 17) as f32 * 0.25 - 2.0)
            .collect()
    }

    /// The state dict pickle: an `OrderedDict` of tensors with `_metadata`.
    fn state_dict(pickle: &mut Pickle, tensors: &[Fake], first_key: usize) {
        pickle.global("collections", "OrderedDict").op(b")R(");
        for (i, tensor) in tensors.iter().enumerate() {
            let class = if tensor.half {
                "HalfStorage"
            } else {
                "FloatStorage"
            };
            let numel = tensor.shape.iter().product();
            let mut stride = vec![1; tensor.shape.len()];
            for d in (0..tensor.shape.len().saturating_sub(1)).rev() {
                stride[d] = stride[d + 1] * tensor.shape[d + 1];
            }
            pickle
                .str(&tensor.name)
                .global("torch._utils", "_rebuild_tensor_v2");
            pickle.op(b"((").str("storage").global("torch", class);
            pickle
                .str(&(first_key + i).to_string())
                .str("cpu")
                .int(numel);
            pickle.op(b"tQ").int(0).tuple(&tensor.shape).tuple(&stride);
            pickle
                .op(b"\x89")
                .global("collections", "OrderedDict")
                .op(b")Rt")
                .op(b"R");
        }
        pickle.op(b"u");
        // `_metadata`, set with BUILD; memoised and recalled like torch does.
        pickle.op(b"}q\x00").str("_metadata").op(b"h\x00s").op(b"b");
    }

    /// A `.pth` file with the state dict under `wrapper` (or bare).
    fn checkpoint(tensors: &[Fake], wrapper: Option<&str>) -> Vec<u8> {
        let mut pickle = Pickle::default();
        pickle.op(b"\x80\x02");
        if let Some(wrapper) = wrapper {
            pickle.op(b"}").str(wrapper);
        }
        state_dict(&mut pickle, tensors, 0);
        if wrapper.is_some() {
            pickle.op(b"s");
        }
        pickle.op(b".");
        let mut entries = vec![
            ("archive/data.pkl".to_owned(), pickle.0),
            ("archive/version".to_owned(), b"3\n".to_vec()),
        ];
        for (i, tensor) in tensors.iter().enumerate() {
            let data = values(i, tensor.shape.iter().product());
            let bytes = if tensor.half {
                data.iter()
                    .flat_map(|&v| f32_to_f16(v).to_le_bytes())
                    .collect()
            } else {
                data.iter().flat_map(|v| v.to_le_bytes()).collect()
            };
            entries.push((format!("archive/data/{i}"), bytes));
        }
        write_zip(&entries)
    }

    fn write_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(name.as_str(), options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// Every entry of a zip, in order.
    fn entries(pth: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(Cursor::new(pth)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut entry = archive.by_index(i).unwrap();
                let name = entry.name().unwrap().into_owned();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                (name, data)
            })
            .collect()
    }

    /// Replace (or add) one zip entry.
    fn with_entry(pth: &[u8], name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut entries: Vec<_> = entries(pth)
            .into_iter()
            .filter(|(entry, _)| entry != name)
            .collect();
        entries.push((name.to_owned(), bytes.to_vec()));
        write_zip(&entries)
    }

    const TINY_PARAM: &str = "\
7767517
10 11
Input                    input                    0 1 data
Split                    splitncnn_input0         1 2 data data_splitncnn_0 data_splitncnn_1
Convolution              conv_0                   1 1 data_splitncnn_1 conv_0 0=2 1=3 4=1 5=1 6=54
PReLU                    prelu_0                  1 1 conv_0 prelu_0 0=2
Convolution              conv_1                   1 1 prelu_0 conv_1 0=2 1=3 4=1 5=1 6=36
PReLU                    prelu_1                  1 1 conv_1 prelu_1 0=2
Convolution              conv_2                   1 1 prelu_1 conv_2 0=12 1=3 4=1 5=1 6=216
PixelShuffle             pixel_shuffle            1 1 conv_2 pixel_shuffle 0=2
Interp                   upsample                 1 1 data_splitncnn_0 upsample 0=1 1=2.000000e+00 2=2.000000e+00
BinaryOp                 add                      2 1 pixel_shuffle upsample output
";

    #[test]
    fn converts_a_compact_checkpoint() {
        let model = compact_to_ncnn(&checkpoint(&tiny(), Some("params"))).unwrap();
        assert_eq!(model.param, TINY_PARAM);

        // Weights in layer order: tagged halves for kernels, floats otherwise.
        let mut expected = Vec::new();
        for (i, tensor) in tiny().iter().enumerate() {
            let data = values(i, tensor.shape.iter().product());
            if tensor.shape.len() == 4 {
                write_half_weights(&mut expected, &data);
            } else {
                write_floats(&mut expected, &data);
            }
        }
        assert_eq!(
            model.bin.len(),
            (4 + 108 + 8) + 8 + (4 + 72 + 8) + 8 + (4 + 432 + 48)
        );
        assert_eq!(model.bin, expected);
        assert_eq!(model.bin[..4], [0x47, 0x6B, 0x30, 0x01]);
        let first = u16::from_le_bytes([model.bin[4], model.bin[5]]);
        assert_eq!(f16_to_f32(first), values(0, 1)[0]);
    }

    #[test]
    fn reads_half_storages_and_any_wrapper() {
        let expected = compact_to_ncnn(&checkpoint(&tiny(), Some("params"))).unwrap();
        let half = tiny()
            .into_iter()
            .map(|tensor| Fake {
                half: true,
                ..tensor
            })
            .collect::<Vec<_>>();
        assert_eq!(
            compact_to_ncnn(&checkpoint(&half, Some("params"))).unwrap(),
            expected
        );
        assert_eq!(
            compact_to_ncnn(&checkpoint(&tiny(), Some("params_ema"))).unwrap(),
            expected
        );
        assert_eq!(
            compact_to_ncnn(&checkpoint(&tiny(), None)).unwrap(),
            expected
        );
    }

    #[test]
    fn prefers_params_ema() {
        // `params` holds a broken net, `params_ema` the good one.
        let mut broken = tiny();
        broken[6].shape = vec![5, 2, 3, 3];
        let mut pickle = Pickle::default();
        pickle.op(b"\x80\x02}(").str("params");
        state_dict(&mut pickle, &broken, 100);
        pickle.str("params_ema");
        state_dict(&mut pickle, &tiny(), 0);
        pickle.op(b"u.");
        let pth = checkpoint(&tiny(), None);
        let pth = with_entry(&pth, "archive/data.pkl", &pickle.0);
        let expected = compact_to_ncnn(&checkpoint(&tiny(), None)).unwrap();
        assert_eq!(compact_to_ncnn(&pth).unwrap(), expected);
    }

    #[test]
    fn infers_the_size_from_the_shapes() {
        // 3 features, three body convolutions, 3×.
        let mut tensors = vec![
            fake("body.0.weight", &[3, 3, 3, 3]),
            fake("body.0.bias", &[3]),
        ];
        for i in 1..=7 {
            if i % 2 == 1 {
                tensors.push(fake(&format!("body.{i}.weight"), &[3]));
            } else {
                tensors.push(fake(&format!("body.{i}.weight"), &[3, 3, 3, 3]));
                tensors.push(fake(&format!("body.{i}.bias"), &[3]));
            }
        }
        tensors.push(fake("body.8.weight", &[27, 3, 3, 3]));
        tensors.push(fake("body.8.bias", &[27]));
        let model = compact_to_ncnn(&checkpoint(&tensors, Some("params"))).unwrap();
        let lines: Vec<&str> = model.param.lines().collect();
        assert_eq!(lines[1], "14 15");
        assert_eq!(model.param.matches("Convolution").count(), 5);
        assert!(model.param.contains(" 0=27 1=3 4=1 5=1 6=729\n"));
        assert!(model.param.contains(" 0=3\nInterp"));
        assert!(model.param.contains(" 0=1 1=3.000000e+00 2=3.000000e+00\n"));
    }

    #[test]
    fn rejects_what_is_not_a_checkpoint() {
        assert_eq!(
            compact_to_ncnn(b"").unwrap_err(),
            "not a PyTorch checkpoint"
        );
        assert_eq!(
            compact_to_ncnn(b"\x80\x02}q\x00.").unwrap_err(),
            "not a PyTorch checkpoint"
        );
        let other = write_zip(&[("hello.txt".to_owned(), b"hi".to_vec())]);
        assert_eq!(
            compact_to_ncnn(&other).unwrap_err(),
            "not a PyTorch checkpoint"
        );
        let pth = checkpoint(&tiny(), Some("params"));
        assert_eq!(
            compact_to_ncnn(&pth[..pth.len() / 2]).unwrap_err(),
            "not a PyTorch checkpoint"
        );
    }

    #[test]
    fn rejects_broken_pickles() {
        let pth = checkpoint(&tiny(), Some("params"));
        let (_, pickle) = entries(&pth)
            .into_iter()
            .find(|(name, _)| name == "archive/data.pkl")
            .unwrap();
        let convert =
            |pickle: &[u8]| compact_to_ncnn(&with_entry(&pth, "archive/data.pkl", pickle));

        for cut in [1, 10, 100, pickle.len() / 2, pickle.len() - 1] {
            assert_eq!(
                convert(&pickle[..cut]).unwrap_err(),
                "truncated pickle",
                "cut at {cut}"
            );
        }
        assert_eq!(
            convert(b"\x80\x02\xff").unwrap_err(),
            "unsupported pickle opcode 0xff"
        );
        assert_eq!(convert(b".").unwrap_err(), "pickle stack underflow");
        assert_eq!(convert(b"(.").unwrap_err(), "pickle stack underflow");
        assert_eq!(
            convert(b"h\x05.").unwrap_err(),
            "pickle refers to a missing memo entry"
        );
        assert_eq!(
            convert(b"cos\nsystem\n.").unwrap_err(),
            "unsupported object in checkpoint: os.system"
        );
        assert_eq!(
            convert(b"K\x01.").unwrap_err(),
            "checkpoint is not a state dict"
        );

        // Deep nesting must neither recurse nor blow up.
        let mut deep = vec![b')'];
        deep.extend(std::iter::repeat_n(0x85, 200_000));
        deep.push(b'.');
        assert_eq!(
            convert(&deep).unwrap_err(),
            "checkpoint is not a state dict"
        );
        // Neither may a self-referencing dict.
        assert_eq!(
            convert(b"}q\x00h\x00h\x00s.").unwrap_err(),
            "unexpected entry in state dict"
        );
    }

    #[test]
    fn rejects_other_layouts() {
        let convert = |tensors: &[Fake]| compact_to_ncnn(&checkpoint(tensors, Some("params")));
        let layout = |tensors: &[Fake]| {
            let err = convert(tensors).unwrap_err();
            assert!(err.starts_with("unexpected layer layout"), "{err}");
        };

        let mut last = tiny();
        last[6].shape = vec![5, 2, 3, 3];
        layout(&last);

        let mut kernel = tiny();
        kernel[3].shape = vec![2, 2, 1, 1];
        layout(&kernel);

        let mut input = tiny();
        input[0].shape = vec![2, 1, 3, 3];
        layout(&input);

        let mut prelu = tiny();
        prelu[2].shape = vec![3];
        layout(&prelu);

        let mut rrdb = tiny();
        rrdb.push(fake("conv_first.weight", &[2, 3, 3, 3]));
        layout(&rrdb);

        let mut gap = tiny();
        gap[6].name = "body.6.weight".to_owned();
        gap[7].name = "body.6.bias".to_owned();
        layout(&gap);

        let mut missing = tiny();
        missing.remove(4);
        layout(&missing);

        layout(&tiny()[..2]);
    }

    #[test]
    fn rejects_bad_storages() {
        let pth = checkpoint(&tiny(), Some("params"));

        let short = with_entry(&pth, "archive/data/3", &[0; 16]);
        assert_eq!(
            compact_to_ncnn(&short).unwrap_err(),
            "tensor data is truncated"
        );

        let missing: Vec<_> = entries(&pth)
            .into_iter()
            .filter(|(name, _)| name != "archive/data/5")
            .collect();
        assert_eq!(
            compact_to_ncnn(&write_zip(&missing)).unwrap_err(),
            "checkpoint has no archive/data/5"
        );

        let big_endian = with_entry(&pth, "archive/byteorder", b"big");
        assert_eq!(
            compact_to_ncnn(&big_endian).unwrap_err(),
            "big-endian checkpoints are not supported"
        );
        let little = with_entry(&pth, "archive/byteorder", b"little");
        assert!(compact_to_ncnn(&little).is_ok());
    }

    #[test]
    fn converts_halves_like_ncnn() {
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(-2.0), 0xC000);
        assert_eq!(f32_to_f16(65504.0), 0x7BFF);
        assert_eq!(f32_to_f16(1e6), 0x7C00);
        assert_eq!(f32_to_f16(f32::NEG_INFINITY), 0xFC00);
        assert_eq!(f32_to_f16(f32::NAN) & 0x7E00, 0x7E00);
        // Truncated, not rounded; too small for a normal half → zero.
        assert_eq!(f32_to_f16(1.0 + 0.75 / 1024.0), 0x3C00);
        assert_eq!(f32_to_f16(1e-6), 0);
        assert_eq!(f32_to_f16(-1e-6), 0x8000);
        for half in (0..=0xFFFFu16).filter(|h| h & 0x7C00 != 0 && h & 0x7FFF <= 0x7C00) {
            assert_eq!(f32_to_f16(f16_to_f32(half)), half, "{half:#06x}");
        }
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
        assert!(f16_to_f32(0x7E00).is_nan());
    }

    #[test]
    fn writes_floats_like_c() {
        assert_eq!(c_float(4.0), "4.000000e+00");
        assert_eq!(c_float(0.5), "5.000000e-01");
        assert_eq!(c_float(12.0), "1.200000e+01");
    }
}
