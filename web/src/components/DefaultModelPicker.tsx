import { lazy, Suspense } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle } from "lucide-react";
import { fetchModels, fetchSettings, updateSettings } from "../api/settings";
const ModelPicker = lazy(() => import("./ModelPicker").then((module) => ({ default: module.ModelPicker })));

export function DefaultModelPicker({ isDisabled = false }: { isDisabled?: boolean }) {
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
  const models = useQuery({ queryKey: ["models"], queryFn: fetchModels });
  const save = useMutation({
    mutationFn: (patch: Parameters<typeof updateSettings>[0]) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to change settings.");
      return updateSettings(patch);
    },
    onSuccess: (next) => queryClient.setQueryData(["settings"], next),
  });

  if (settings.isError || models.isError) return <span className="model-control-error" title="Could not load model settings"><AlertCircle size={12} /> Model unavailable</span>;
  if (settings.isLoading || models.isLoading) return <span className="model-control-loading">Loading models…</span>;

  const current = settings.data!;
  return <div className="default-model-control">
    <Suspense fallback={<span className="model-control-loading">Choose a model…</span>}><ModelPicker
      models={models.data?.items ?? []}
      value={current.default_model}
      favorites={current.favorite_models}
      isDisabled={isDisabled || save.isPending}
      placeholder="Choose a model"
      onChange={(default_model) => save.mutate({ default_model })}
      onToggleFavorite={(id) => {
        const favorite_models = current.favorite_models.includes(id)
          ? current.favorite_models.filter((item) => item !== id)
          : [...current.favorite_models, id];
        save.mutate({ favorite_models });
      }}
    /></Suspense>
    {save.isPending && <span className="model-saving">Saving…</span>}
  </div>;
}
