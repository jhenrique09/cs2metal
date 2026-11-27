// F1 probe: can a Vulkan driver run inside this process and present to a window?
//
// Opens a window through SDL3, creates a Vulkan instance, device, and swapchain
// with whichever driver the loader is given (VK_DRIVER_FILES), creates one
// graphics pipeline from SPIR-V to time shader translation, then clears and
// presents swapchain images for a fixed number of frames and times them.
//
// Built for x86_64 it runs under Rosetta 2, the way the cs2metal host will.
// Prints one "key: value" line per fact and exits non-zero naming the failed step.

#include <SDL3/SDL.h>
#include <SDL3/SDL_vulkan.h>
#include <vulkan/vulkan.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysctl.h>
#include <time.h>

#define FRAMES 300

static double now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static void fail(const char *step, VkResult r) {
    printf("result: failed at %s (VkResult %d)\n", step, r);
    exit(2);
}

#define CHECK(step, call)                                                                   \
    do {                                                                                    \
        VkResult r_ = (call);                                                               \
        if (r_ != VK_SUCCESS) fail(step, r_);                                               \
    } while (0)

#if defined(__x86_64__)
static int translated(void) {
    int value = 0;
    size_t size = sizeof value;
    if (sysctlbyname("sysctl.proc_translated", &value, &size, NULL, 0) != 0) return 0;
    return value;
}
#endif

static void *read_file(const char *path, size_t *size) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    void *data = malloc((size_t)n);
    if (data && fread(data, 1, (size_t)n, f) != (size_t)n) {
        free(data);
        data = NULL;
    }
    fclose(f);
    *size = (size_t)n;
    return data;
}

static int has_extension(const VkExtensionProperties *list, uint32_t count, const char *name) {
    for (uint32_t i = 0; i < count; i++)
        if (strcmp(list[i].extensionName, name) == 0) return 1;
    return 0;
}

static VkShaderModule shader(VkDevice device, const char *path) {
    size_t size = 0;
    void *code = read_file(path, &size);
    if (!code) {
        printf("result: failed at reading %s\n", path);
        exit(2);
    }
    VkShaderModuleCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        .codeSize = size,
        .pCode = code,
    };
    VkShaderModule module;
    CHECK("vkCreateShaderModule", vkCreateShaderModule(device, &info, NULL, &module));
    free(code);
    return module;
}

int main(int argc, char **argv) {
    if (argc != 4) {
        fprintf(stderr, "usage: %s <libvulkan> <vert.spv> <frag.spv>\n", argv[0]);
        return 1;
    }
#if defined(__x86_64__)
    printf("arch: x86_64%s\n", translated() ? " (Rosetta 2)" : "");
#else
    printf("arch: arm64\n");
#endif

    if (!SDL_Init(SDL_INIT_VIDEO)) {
        printf("result: failed at SDL_Init: %s\n", SDL_GetError());
        return 2;
    }
    if (!SDL_Vulkan_LoadLibrary(argv[1])) {
        printf("result: failed at SDL_Vulkan_LoadLibrary: %s\n", SDL_GetError());
        return 2;
    }
    SDL_Window *window = SDL_CreateWindow("cs2metal F1 probe", 640, 360, SDL_WINDOW_VULKAN);
    if (!window) {
        printf("result: failed at SDL_CreateWindow: %s\n", SDL_GetError());
        return 2;
    }

    // Instance: SDL's surface extensions, plus portability enumeration when the
    // loader offers it, which MoltenVK, a portability driver, needs.
    Uint32 sdl_count = 0;
    const char *const *sdl_ext = SDL_Vulkan_GetInstanceExtensions(&sdl_count);
    uint32_t avail_count = 0;
    vkEnumerateInstanceExtensionProperties(NULL, &avail_count, NULL);
    VkExtensionProperties *avail = calloc(avail_count, sizeof *avail);
    vkEnumerateInstanceExtensionProperties(NULL, &avail_count, avail);
    const char *inst_ext[16];
    uint32_t n_inst = 0;
    for (Uint32 i = 0; i < sdl_count && n_inst < 15; i++) inst_ext[n_inst++] = sdl_ext[i];
    VkInstanceCreateFlags flags = 0;
    if (has_extension(avail, avail_count, VK_KHR_PORTABILITY_ENUMERATION_EXTENSION_NAME)) {
        inst_ext[n_inst++] = VK_KHR_PORTABILITY_ENUMERATION_EXTENSION_NAME;
        flags |= VK_INSTANCE_CREATE_ENUMERATE_PORTABILITY_BIT_KHR;
    }
    VkApplicationInfo app = {
        .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
        .pApplicationName = "cs2metal-f1",
        .apiVersion = VK_API_VERSION_1_3,
    };
    VkInstanceCreateInfo inst_info = {
        .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
        .flags = flags,
        .pApplicationInfo = &app,
        .enabledExtensionCount = n_inst,
        .ppEnabledExtensionNames = inst_ext,
    };
    VkInstance instance;
    CHECK("vkCreateInstance", vkCreateInstance(&inst_info, NULL, &instance));

    uint32_t gpu_count = 0;
    vkEnumeratePhysicalDevices(instance, &gpu_count, NULL);
    if (gpu_count == 0) fail("vkEnumeratePhysicalDevices (no device)", VK_ERROR_INITIALIZATION_FAILED);
    VkPhysicalDevice gpu;
    gpu_count = 1;
    vkEnumeratePhysicalDevices(instance, &gpu_count, &gpu);
    VkPhysicalDeviceDriverProperties driver = {.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES};
    VkPhysicalDeviceProperties2 props = {.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2, .pNext = &driver};
    vkGetPhysicalDeviceProperties2(gpu, &props);
    printf("device: %s\n", props.properties.deviceName);
    printf("driver: %s %s\n", driver.driverName, driver.driverInfo);

    VkSurfaceKHR surface;
    if (!SDL_Vulkan_CreateSurface(window, instance, NULL, &surface)) {
        printf("result: failed at SDL_Vulkan_CreateSurface: %s\n", SDL_GetError());
        return 2;
    }

    uint32_t family_count = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(gpu, &family_count, NULL);
    VkQueueFamilyProperties *families = calloc(family_count, sizeof *families);
    vkGetPhysicalDeviceQueueFamilyProperties(gpu, &family_count, families);
    uint32_t family = UINT32_MAX;
    for (uint32_t i = 0; i < family_count && family == UINT32_MAX; i++) {
        VkBool32 present = VK_FALSE;
        vkGetPhysicalDeviceSurfaceSupportKHR(gpu, i, surface, &present);
        if ((families[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) && present) family = i;
    }
    if (family == UINT32_MAX) fail("finding a graphics and present queue", VK_ERROR_FEATURE_NOT_PRESENT);

    // Device: swapchain, plus the portability subset when the driver has it.
    uint32_t dev_ext_count = 0;
    vkEnumerateDeviceExtensionProperties(gpu, NULL, &dev_ext_count, NULL);
    VkExtensionProperties *dev_avail = calloc(dev_ext_count, sizeof *dev_avail);
    vkEnumerateDeviceExtensionProperties(gpu, NULL, &dev_ext_count, dev_avail);
    const char *dev_ext[2] = {VK_KHR_SWAPCHAIN_EXTENSION_NAME};
    uint32_t n_dev = 1;
    if (has_extension(dev_avail, dev_ext_count, "VK_KHR_portability_subset"))
        dev_ext[n_dev++] = "VK_KHR_portability_subset";
    float priority = 1.0f;
    VkDeviceQueueCreateInfo queue_info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
        .queueFamilyIndex = family,
        .queueCount = 1,
        .pQueuePriorities = &priority,
    };
    VkPhysicalDeviceVulkan13Features v13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        .dynamicRendering = VK_TRUE,
    };
    VkDeviceCreateInfo dev_info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
        .pNext = &v13,
        .queueCreateInfoCount = 1,
        .pQueueCreateInfos = &queue_info,
        .enabledExtensionCount = n_dev,
        .ppEnabledExtensionNames = dev_ext,
    };
    VkDevice device;
    CHECK("vkCreateDevice", vkCreateDevice(gpu, &dev_info, NULL, &device));
    VkQueue queue;
    vkGetDeviceQueue(device, family, 0, &queue);

    // Shader translation: one pipeline from SPIR-V, timed from module to pipeline.
    double t0 = now_ms();
    VkShaderModule vert = shader(device, argv[2]);
    VkShaderModule frag = shader(device, argv[3]);
    VkFormat color_format = VK_FORMAT_B8G8R8A8_UNORM;
    VkPipelineShaderStageCreateInfo stages[2] = {
        {.sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
         .stage = VK_SHADER_STAGE_VERTEX_BIT, .module = vert, .pName = "main"},
        {.sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
         .stage = VK_SHADER_STAGE_FRAGMENT_BIT, .module = frag, .pName = "main"},
    };
    VkPipelineVertexInputStateCreateInfo vertex_input = {.sType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO};
    VkPipelineInputAssemblyStateCreateInfo assembly = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
        .topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
    };
    VkPipelineViewportStateCreateInfo viewport = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
        .viewportCount = 1,
        .scissorCount = 1,
    };
    VkPipelineRasterizationStateCreateInfo raster = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
        .polygonMode = VK_POLYGON_MODE_FILL,
        .cullMode = VK_CULL_MODE_NONE,
        .lineWidth = 1.0f,
    };
    VkPipelineMultisampleStateCreateInfo multisample = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
        .rasterizationSamples = VK_SAMPLE_COUNT_1_BIT,
    };
    VkPipelineColorBlendAttachmentState blend_attachment = {.colorWriteMask = 0xf};
    VkPipelineColorBlendStateCreateInfo blend = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
        .attachmentCount = 1,
        .pAttachments = &blend_attachment,
    };
    VkDynamicState dynamic_states[2] = {VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR};
    VkPipelineDynamicStateCreateInfo dynamic = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
        .dynamicStateCount = 2,
        .pDynamicStates = dynamic_states,
    };
    VkPipelineRenderingCreateInfo rendering = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO,
        .colorAttachmentCount = 1,
        .pColorAttachmentFormats = &color_format,
    };
    VkPipelineLayoutCreateInfo layout_info = {.sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
    VkPipelineLayout layout;
    CHECK("vkCreatePipelineLayout", vkCreatePipelineLayout(device, &layout_info, NULL, &layout));
    VkGraphicsPipelineCreateInfo pipeline_info = {
        .sType = VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
        .pNext = &rendering,
        .stageCount = 2,
        .pStages = stages,
        .pVertexInputState = &vertex_input,
        .pInputAssemblyState = &assembly,
        .pViewportState = &viewport,
        .pRasterizationState = &raster,
        .pMultisampleState = &multisample,
        .pColorBlendState = &blend,
        .pDynamicState = &dynamic,
        .layout = layout,
    };
    VkPipeline pipeline;
    CHECK("vkCreateGraphicsPipelines",
          vkCreateGraphicsPipelines(device, VK_NULL_HANDLE, 1, &pipeline_info, NULL, &pipeline));
    printf("pipeline creation ms: %.2f\n", now_ms() - t0);

    // Swapchain.
    VkSurfaceCapabilitiesKHR caps;
    CHECK("vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
          vkGetPhysicalDeviceSurfaceCapabilitiesKHR(gpu, surface, &caps));
    VkExtent2D extent = caps.currentExtent;
    if (extent.width == UINT32_MAX) extent = (VkExtent2D){640, 360};
    uint32_t min_images = caps.minImageCount + 1;
    if (caps.maxImageCount && min_images > caps.maxImageCount) min_images = caps.maxImageCount;
    VkSwapchainCreateInfoKHR sc_info = {
        .sType = VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
        .surface = surface,
        .minImageCount = min_images,
        .imageFormat = color_format,
        .imageColorSpace = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR,
        .imageExtent = extent,
        .imageArrayLayers = 1,
        .imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        .preTransform = caps.currentTransform,
        .compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR,
        .presentMode = VK_PRESENT_MODE_FIFO_KHR,
        .clipped = VK_TRUE,
    };
    VkSwapchainKHR swapchain;
    CHECK("vkCreateSwapchainKHR", vkCreateSwapchainKHR(device, &sc_info, NULL, &swapchain));
    uint32_t image_count = 0;
    vkGetSwapchainImagesKHR(device, swapchain, &image_count, NULL);
    VkImage *images = calloc(image_count, sizeof *images);
    vkGetSwapchainImagesKHR(device, swapchain, &image_count, images);
    printf("swapchain: %ux%u, %u images, FIFO\n", extent.width, extent.height, image_count);

    VkCommandPoolCreateInfo pool_info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        .flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
        .queueFamilyIndex = family,
    };
    VkCommandPool pool;
    CHECK("vkCreateCommandPool", vkCreateCommandPool(device, &pool_info, NULL, &pool));
    VkCommandBufferAllocateInfo alloc = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        .commandPool = pool,
        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        .commandBufferCount = 1,
    };
    VkCommandBuffer cmd;
    CHECK("vkAllocateCommandBuffers", vkAllocateCommandBuffers(device, &alloc, &cmd));
    VkSemaphoreCreateInfo sem_info = {.sType = VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};
    VkSemaphore acquired, *rendered = calloc(image_count, sizeof *rendered);
    CHECK("vkCreateSemaphore", vkCreateSemaphore(device, &sem_info, NULL, &acquired));
    for (uint32_t i = 0; i < image_count; i++)
        CHECK("vkCreateSemaphore", vkCreateSemaphore(device, &sem_info, NULL, &rendered[i]));
    VkFenceCreateInfo fence_info = {.sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    VkFence fence;
    CHECK("vkCreateFence", vkCreateFence(device, &fence_info, NULL, &fence));

    // Frames: clear each acquired image to a colour that changes, and present.
    double first = 0, start = now_ms(), worst = 0, last = start;
    for (int frame = 0; frame < FRAMES; frame++) {
        SDL_Event e;
        while (SDL_PollEvent(&e)) {
        }
        uint32_t index;
        CHECK("vkAcquireNextImageKHR",
              vkAcquireNextImageKHR(device, swapchain, UINT64_MAX, acquired, VK_NULL_HANDLE, &index));
        VkCommandBufferBeginInfo begin = {
            .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
        };
        CHECK("vkBeginCommandBuffer", vkBeginCommandBuffer(cmd, &begin));
        VkImageSubresourceRange range = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
        VkImageMemoryBarrier to_clear = {
            .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            .dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT,
            .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
            .newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
            .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .image = images[index],
            .subresourceRange = range,
        };
        vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
                             0, 0, NULL, 0, NULL, 1, &to_clear);
        float t = (float)frame / FRAMES;
        VkClearColorValue color = {.float32 = {0.1f, 0.2f + 0.6f * t, 0.5f, 1.0f}};
        vkCmdClearColorImage(cmd, images[index], VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &color, 1, &range);
        VkImageMemoryBarrier to_present = to_clear;
        to_present.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
        to_present.dstAccessMask = 0;
        to_present.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        to_present.newLayout = VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;
        vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                             0, 0, NULL, 0, NULL, 1, &to_present);
        CHECK("vkEndCommandBuffer", vkEndCommandBuffer(cmd));
        VkPipelineStageFlags wait = VK_PIPELINE_STAGE_TRANSFER_BIT;
        VkSubmitInfo submit = {
            .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
            .waitSemaphoreCount = 1,
            .pWaitSemaphores = &acquired,
            .pWaitDstStageMask = &wait,
            .commandBufferCount = 1,
            .pCommandBuffers = &cmd,
            .signalSemaphoreCount = 1,
            .pSignalSemaphores = &rendered[index],
        };
        CHECK("vkQueueSubmit", vkQueueSubmit(queue, 1, &submit, fence));
        VkPresentInfoKHR present = {
            .sType = VK_STRUCTURE_TYPE_PRESENT_INFO_KHR,
            .waitSemaphoreCount = 1,
            .pWaitSemaphores = &rendered[index],
            .swapchainCount = 1,
            .pSwapchains = &swapchain,
            .pImageIndices = &index,
        };
        CHECK("vkQueuePresentKHR", vkQueuePresentKHR(queue, &present));
        CHECK("vkWaitForFences", vkWaitForFences(device, 1, &fence, VK_TRUE, UINT64_MAX));
        CHECK("vkResetFences", vkResetFences(device, 1, &fence));
        double t_now = now_ms();
        if (frame == 0) first = t_now - start;
        else if (t_now - last > worst) worst = t_now - last;
        last = t_now;
    }
    double total = now_ms() - start;
    printf("first frame ms: %.2f\n", first);
    printf("frames: %d, mean frame ms: %.2f, worst frame ms: %.2f\n", FRAMES,
           (total - first) / (FRAMES - 1), worst);

    vkDeviceWaitIdle(device);
    vkDestroyFence(device, fence, NULL);
    for (uint32_t i = 0; i < image_count; i++) vkDestroySemaphore(device, rendered[i], NULL);
    vkDestroySemaphore(device, acquired, NULL);
    vkDestroyCommandPool(device, pool, NULL);
    vkDestroySwapchainKHR(device, swapchain, NULL);
    vkDestroyPipeline(device, pipeline, NULL);
    vkDestroyPipelineLayout(device, layout, NULL);
    vkDestroyShaderModule(device, vert, NULL);
    vkDestroyShaderModule(device, frag, NULL);
    vkDestroyDevice(device, NULL);
    vkDestroySurfaceKHR(instance, surface, NULL);
    vkDestroyInstance(instance, NULL);
    SDL_DestroyWindow(window);
    SDL_Quit();
    printf("result: presented\n");
    return 0;
}
