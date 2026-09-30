import { lazy, Suspense } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle } from "lucide-react";
import { fetchModels, fetchSettings, updateSettingsAndRefetchFavorites } from "../api/settings";
import type { Settings, SettingsPatch } from "../api/settings";
const ModelPicker = lazy(() => import("./ModelPicker").then((module) => ({ default: module.ModelPicker })));

export function DefaultModelPicker({ isDisabled = false }: { isDisabled?: boolean }) {
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
  const models = useQuery({ queryKey: ["models"], queryFn: fetchModels });
  const save = useMutation({
    mutationFn: (variables: { patch: SettingsPatch; previous: Settings | undefined }) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to change settings.");
      return updateSettingsAndRefetchFavorites(variables.patch);
    },
    onError: (_error, variables) => {
      if (variables.previous) queryClient.setQueryData(["settings"], variables.previous);
    },
    onSuccess: (updatedSettings) => {
      queryClient.setQueryData<Settings>(["settings"], updatedSettings);
    },
  });

  if (settings.isError || models.isError) return <span className="model-control-error" title="Could not load model settings"><AlertCircle size={12} /> Model unavailable</span>;
  if (settings.isLoading || models.isLoading) return <span className="model-control-loading">Loading models…</span>;

  const current = settings.data!;
  return <div className="w-[min(260px,48vw)] min-w-0">
    <Suspense fallback={<span className="model-control-loading">Choose a model…</span>}><ModelPicker
      models={models.data?.items ?? []}
      value={current.default_model}
      favorites={current.favorite_models}
      isDisabled={isDisabled || save.isPending}
      placeholder="Choose a model"
      onChange={(default_model) => save.mutate({ patch: { default_model }, previous: undefined })}
      onToggleFavorite={(id) => {
        const previous = queryClient.getQueryData<Settings>(["settings"]) ?? current;
        const favorite_models = previous.favorite_models.includes(id)
          ? previous.favorite_models.filter((item) => item !== id)
          : [...previous.favorite_models, id];
        queryClient.setQueryData<Settings>(["settings"], { ...previous, favorite_models });
        save.mutate({ patch: { favorite_models }, previous });
      }}
    /></Suspense>
    {save.isPending && <span className="model-saving">Saving…</span>}
  </div>;
}
