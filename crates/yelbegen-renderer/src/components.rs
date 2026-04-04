use std::sync::Arc;
use yelbegen_math::vec3::Vec3;

pub struct Mesh {
    pub vbuf: Arc<wgpu::Buffer>,
    pub vertex_count: u32,
    pub center_offset: Vec3,
}



impl Mesh {
    pub fn new(vbuf: Arc<wgpu::Buffer>, vertex_count: u32, center_offset: Vec3) -> Self {
        Self { vbuf, vertex_count, center_offset }
    }
}

pub struct Material {
    pub bind_group: Arc<wgpu::BindGroup>,
}



impl Material {
    pub fn new(bind_group: Arc<wgpu::BindGroup>) -> Self {
        Self { bind_group }
    }
}

pub struct MeshRenderer {
    pub ubuf: wgpu::Buffer,
    pub ubind: wgpu::BindGroup,
}



impl MeshRenderer {
    pub fn new(ubuf: wgpu::Buffer, ubind: wgpu::BindGroup) -> Self {
        Self { ubuf, ubind }
    }
}

pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub primary: bool,
}

impl Camera {
    pub fn new(fov: f32, near: f32, far: f32, yaw: f32, pitch: f32, primary: bool) -> Self {
        Self { fov, near, far, yaw, pitch, primary }
    }

    pub fn get_projection(&self, aspect: f32) -> yelbegen_math::mat4::Mat4 {
        yelbegen_math::mat4::Mat4::perspective(self.fov, aspect, self.near, self.far)
    }

    pub fn get_view(&self, position: Vec3) -> yelbegen_math::mat4::Mat4 {
        let front = self.get_front();
        yelbegen_math::mat4::Mat4::look_at_rh(position, position + front, Vec3::new(0.0, 1.0, 0.0))
    }
    
    pub fn get_front(&self) -> Vec3 {
        let fx = self.yaw.cos() * self.pitch.cos();
        let fy = self.pitch.sin();
        let fz = self.yaw.sin() * self.pitch.cos();
        Vec3::new(fx, fy, fz).normalize()
    }
    
    pub fn get_right(&self) -> Vec3 {
        self.get_front().cross(Vec3::new(0.0, 1.0, 0.0)).normalize()
    }
}
